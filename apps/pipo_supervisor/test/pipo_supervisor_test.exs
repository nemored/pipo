defmodule PipoSupervisor.RouterTest do
  use ExUnit.Case, async: false
  import ExUnit.CaptureLog

  defmodule TestWorker do
    use GenServer

    def start_link(test_pid, behavior \\ :ok) do
      GenServer.start_link(__MODULE__, {test_pid, behavior})
    end

    def init({test_pid, behavior}), do: {:ok, %{test_pid: test_pid, behavior: behavior}}

    def set_behavior(pid, behavior), do: GenServer.call(pid, {:set_behavior, behavior})

    def handle_call({:deliver, frame}, _from, %{behavior: :ok} = state) do
      send(state.test_pid, {:delivered, self(), frame})
      {:reply, :ok, state}
    end

    def handle_call({:deliver, _frame}, _from, %{behavior: :sleep} = state) do
      Process.sleep(100)
      {:reply, :ok, state}
    end

    def handle_call({:deliver, _frame}, _from, %{behavior: :sleep_once} = state) do
      Process.sleep(100)
      {:reply, :ok, %{state | behavior: :ok}}
    end

    def handle_call({:set_behavior, behavior}, _from, state) do
      {:reply, :ok, %{state | behavior: behavior}}
    end
  end

  test "builds routing table by inverting channel mapping and removes origin from fanout" do
    {:ok, router} =
      start_supervised(
        {PipoSupervisor.Router,
         name: :router_test_1,
         channel_mapping: %{
           worker_a: ["alpha", "beta"],
           worker_b: ["alpha"]
         }}
      )

    {:ok, worker_a} = TestWorker.start_link(self())
    {:ok, worker_b} = TestWorker.start_link(self())

    assert :ok = PipoSupervisor.Router.register_worker(router, :worker_a, worker_a)
    assert :ok = PipoSupervisor.Router.register_worker(router, :worker_b, worker_b)

    PipoSupervisor.Router.publish(router, :worker_a, "alpha", %{"x" => 1})

    assert_receive {:delivered, ^worker_b,
                    %{"bus" => "alpha", "payload" => %{"x" => 1}, "pipo_id" => 1}},
                   500

    refute_receive {:delivered, ^worker_a, _}, 100
  end

  test "times out deliveries, marks degraded and logs degraded/restored transitions" do
    {:ok, router} =
      start_supervised(
        {PipoSupervisor.Router,
         name: :router_test_2,
         channel_mapping: %{slow: ["alpha"], source: ["alpha"]},
         call_timeout_ms: 10,
         drop_threshold: 1,
         notify_pid: self()}
      )

    {:ok, slow_worker} = TestWorker.start_link(self(), :sleep_once)
    {:ok, source_worker} = TestWorker.start_link(self())

    assert :ok = PipoSupervisor.Router.register_worker(router, :slow, slow_worker)
    assert :ok = PipoSupervisor.Router.register_worker(router, :source, source_worker)

    log =
      capture_log(fn ->
        PipoSupervisor.Router.publish(router, :source, "alpha", %{"step" => 1})
        assert_receive {:worker_degraded, :slow, 1, :timeout}, 500

        Process.sleep(120)
        PipoSupervisor.Router.publish(router, :source, "alpha", %{"step" => 2})
        assert_receive {:worker_restored, :slow}, 500
      end)

    assert log =~ "router worker_degraded worker_id=:slow"
    assert log =~ "router worker_restored worker_id=:slow"
  end

  test "supports dynamic add/remove subscriptions and worker unregister" do
    {:ok, router} =
      start_supervised(
        {PipoSupervisor.Router, name: :router_test_3, channel_mapping: %{source: ["alpha"]}}
      )

    {:ok, source_worker} = TestWorker.start_link(self())
    {:ok, dynamic_worker} = TestWorker.start_link(self())

    assert :ok = PipoSupervisor.Router.register_worker(router, :source, source_worker)
    assert :ok = PipoSupervisor.Router.register_worker(router, :dynamic, dynamic_worker)

    assert :ok = PipoSupervisor.Router.add_subscription(router, :dynamic, "alpha")

    PipoSupervisor.Router.publish(router, :source, "alpha", %{"phase" => "added"})

    assert_receive {:delivered, ^dynamic_worker,
                    %{"bus" => "alpha", "payload" => %{"phase" => "added"}}},
                   500

    assert :ok = PipoSupervisor.Router.remove_subscription(router, :dynamic, "alpha")

    PipoSupervisor.Router.publish(router, :source, "alpha", %{"phase" => "removed"})
    refute_receive {:delivered, ^dynamic_worker, %{"payload" => %{"phase" => "removed"}}}, 150

    assert :ok = PipoSupervisor.Router.unregister_worker(router, :dynamic)
    assert :ok = PipoSupervisor.Router.add_subscription(router, :dynamic, "alpha")

    PipoSupervisor.Router.publish(router, :source, "alpha", %{"phase" => "unregistered"})

    refute_receive {:delivered, ^dynamic_worker, %{"payload" => %{"phase" => "unregistered"}}},
                   150
  end

  test "maintains forward/reverse index integrity across dynamic topology changes" do
    {:ok, router} =
      start_supervised(
        {PipoSupervisor.Router,
         name: :router_test_4, channel_mapping: %{source: ["alpha"], a: ["alpha"], b: ["beta"]}}
      )

    {:ok, source_worker} = TestWorker.start_link(self())
    {:ok, worker_a} = TestWorker.start_link(self())
    {:ok, worker_b} = TestWorker.start_link(self())

    assert :ok = PipoSupervisor.Router.register_worker(router, :source, source_worker)
    assert :ok = PipoSupervisor.Router.register_worker(router, :a, worker_a)
    assert :ok = PipoSupervisor.Router.register_worker(router, :b, worker_b)

    assert_index_integrity(router)

    assert :ok = PipoSupervisor.Router.add_subscription(router, :a, "beta")
    assert :ok = PipoSupervisor.Router.remove_subscription(router, :a, "alpha")

    assert_index_integrity(router)

    assert :ok = PipoSupervisor.Router.unregister_worker(router, :a)

    assert_index_integrity(router)

    state = :sys.get_state(router)
    assert state.routing_table["alpha"] == MapSet.new([:source])
    assert state.routing_table["beta"] == MapSet.new([:b])
    refute Map.has_key?(state.worker_buses, :a)
  end

  test "does not duplicate delivery after remove and re-add of subscription" do
    {:ok, router} =
      start_supervised(
        {PipoSupervisor.Router, name: :router_test_5, channel_mapping: %{source: ["alpha"]}}
      )

    {:ok, source_worker} = TestWorker.start_link(self())
    {:ok, dynamic_worker} = TestWorker.start_link(self())

    assert :ok = PipoSupervisor.Router.register_worker(router, :source, source_worker)
    assert :ok = PipoSupervisor.Router.register_worker(router, :dynamic, dynamic_worker)

    assert :ok = PipoSupervisor.Router.add_subscription(router, :dynamic, "alpha")
    assert :ok = PipoSupervisor.Router.remove_subscription(router, :dynamic, "alpha")
    assert :ok = PipoSupervisor.Router.add_subscription(router, :dynamic, "alpha")

    PipoSupervisor.Router.publish(router, :source, "alpha", %{"phase" => "readd"})

    assert_receive {:delivered, ^dynamic_worker, %{"payload" => %{"phase" => "readd"}}}, 500
    refute_receive {:delivered, ^dynamic_worker, %{"payload" => %{"phase" => "readd"}}}, 150
  end

  test "preserves self-send filtering after subscription topology changes" do
    {:ok, router} =
      start_supervised(
        {PipoSupervisor.Router,
         name: :router_test_6, channel_mapping: %{source: ["alpha"], observer: ["alpha"]}}
      )

    {:ok, source_worker} = TestWorker.start_link(self())
    {:ok, observer_worker} = TestWorker.start_link(self())

    assert :ok = PipoSupervisor.Router.register_worker(router, :source, source_worker)
    assert :ok = PipoSupervisor.Router.register_worker(router, :observer, observer_worker)

    assert :ok = PipoSupervisor.Router.remove_subscription(router, :source, "alpha")
    assert :ok = PipoSupervisor.Router.add_subscription(router, :source, "alpha")

    PipoSupervisor.Router.publish(router, :source, "alpha", %{"msg" => "topology-shift"})

    assert_receive {:delivered, ^observer_worker, %{"payload" => %{"msg" => "topology-shift"}}},
                   500

    refute_receive {:delivered, ^source_worker, _}, 150
  end

  test "degraded handling and thresholds remain local to failing worker" do
    {:ok, router} =
      start_supervised(
        {PipoSupervisor.Router,
         name: :router_test_7,
         channel_mapping: %{source: ["alpha"], healthy: ["alpha"], slow: ["alpha"]},
         call_timeout_ms: 10,
         drop_threshold: 2,
         notify_pid: self()}
      )

    {:ok, source_worker} = TestWorker.start_link(self())
    {:ok, healthy_worker} = TestWorker.start_link(self())
    {:ok, slow_worker} = TestWorker.start_link(self(), :sleep)

    assert :ok = PipoSupervisor.Router.register_worker(router, :source, source_worker)
    assert :ok = PipoSupervisor.Router.register_worker(router, :healthy, healthy_worker)
    assert :ok = PipoSupervisor.Router.register_worker(router, :slow, slow_worker)

    PipoSupervisor.Router.publish(router, :source, "alpha", %{"seq" => 1})
    assert_receive {:delivered, ^healthy_worker, %{"payload" => %{"seq" => 1}}}, 500
    refute_receive {:worker_degraded, :slow, 1, :timeout}, 100

    PipoSupervisor.Router.publish(router, :source, "alpha", %{"seq" => 2})
    assert_receive {:delivered, ^healthy_worker, %{"payload" => %{"seq" => 2}}}, 500
    assert_receive {:worker_degraded, :slow, 2, :timeout}, 500
    refute_receive {:worker_degraded, :healthy, _drops, _reason}, 100
  end

  defp assert_index_integrity(router) do
    state = :sys.get_state(router)

    Enum.each(state.routing_table, fn {bus, workers} ->
      assert map_size(workers) > 0

      Enum.each(workers, fn worker_id ->
        assert bus in Map.get(state.worker_buses, worker_id, MapSet.new())
      end)
    end)

    Enum.each(state.worker_buses, fn {worker_id, buses} ->
      assert map_size(buses) > 0

      Enum.each(buses, fn bus ->
        assert worker_id in Map.get(state.routing_table, bus, MapSet.new())
      end)
    end)
  end
end

defmodule PipoSupervisor.ChaosTest do
  use ExUnit.Case, async: false
  import ExUnit.CaptureLog

  alias PipoSupervisor.RouterTest.TestWorker

  setup do
    :persistent_term.put({PipoSupervisor.PortWorker, :restart_counts}, %{})
    :ok
  end

  test "restart storm enforces supervisor intensity and logs restart/backoff metadata" do
    crash_transport = write_transport!("crash_transport.sh", crash_transport_script())

    Process.flag(:trap_exit, true)

    log =
      capture_log(fn ->
        sup_name = unique_name(:chaos_supervisor_intensity)

        {:ok, supervisor_pid} =
          PipoSupervisor.Supervisor.start_link(
            name: sup_name,
            workers: [:storm],
            max_restarts: 2,
            max_seconds: 1,
            router: [name: unique_name(:chaos_router_intensity)],
            port_worker: [
              transport_path: crash_transport,
              backoff_ms: 5,
              jitter_ms: 5,
              ready_timeout_ms: 25,
              shutdown_timeout_ms: 20
            ]
          )

        ref = Process.monitor(supervisor_pid)
        assert_receive {:DOWN, ^ref, :process, ^supervisor_pid, _reason}, 1_500
      end)

    assert log =~ "instance_id=storm-0"
    assert log =~ "instance_id=storm-1"
    assert log =~ "transport="
    assert log =~ "crash_transport.sh"
    assert log =~ "restart_count=0"
    assert log =~ "restart_count=1"
    assert log =~ "backoff_ms="
  end

  test "one_for_one restart keeps healthy worker and router delivery available" do
    crash_transport = write_transport!("storm_transport.sh", crash_transport_script())
    healthy_transport = write_transport!("healthy_transport.sh", healthy_transport_script())

    router = unique_name(:chaos_router_availability)

    {:ok, _supervisor_pid} =
      start_supervised(
        {PipoSupervisor.Supervisor,
         name: unique_name(:chaos_supervisor_router),
         workers: [:healthy, :storm],
         router: [name: router, channel_mapping: %{healthy: ["alpha"], observer: ["alpha"]}],
         max_restarts: 100,
         max_seconds: 1,
         port_worker: [backoff_ms: 5, jitter_ms: 5, ready_timeout_ms: 50],
         worker_overrides: %{
           healthy: [transport_path: healthy_transport],
           storm: [transport_path: crash_transport]
         }}
      )

    {:ok, observer_worker} = TestWorker.start_link(self())

    assert :ok = PipoSupervisor.Router.register_worker(router, :observer, observer_worker)

    healthy_pid = wait_for_worker(:healthy)
    storm_pid_before = wait_for_worker(:storm)

    PipoSupervisor.Router.publish(router, :healthy, "alpha", %{"seq" => 1})

    assert_receive {:delivered, ^observer_worker, %{"payload" => %{"seq" => 1}}}, 500

    Process.sleep(200)

    storm_pid_after = wait_for_worker(:storm)
    assert storm_pid_before != storm_pid_after
    assert healthy_pid == wait_for_worker(:healthy)

    PipoSupervisor.Router.publish(router, :healthy, "alpha", %{"seq" => 2})

    assert_receive {:delivered, ^observer_worker, %{"payload" => %{"seq" => 2}}}, 500
  end

  defp wait_for_worker(id, attempts \\ 40)

  defp wait_for_worker(_id, 0), do: flunk("worker did not register")

  defp wait_for_worker(id, attempts) do
    case GenServer.whereis(PipoSupervisor.PortWorker.via_name(id)) do
      nil ->
        Process.sleep(25)
        wait_for_worker(id, attempts - 1)

      pid ->
        pid
    end
  end

  defp write_transport!(name, script) do
    path = Path.join(System.tmp_dir!(), "pipo_#{System.unique_integer([:positive])}_#{name}")
    File.write!(path, script)
    File.chmod!(path, 0o755)
    path
  end

  defp crash_transport_script do
    "#!/bin/sh
printf '{\"event\":\"ready\"}\n'
exit 1
"
  end

  defp healthy_transport_script do
    "#!/bin/sh
printf '{\"event\":\"ready\"}\n'
while IFS= read -r _line; do
  :
done
"
  end

  defp unique_name(prefix) do
    String.to_atom("#{prefix}_#{System.unique_integer([:positive])}")
  end
end
