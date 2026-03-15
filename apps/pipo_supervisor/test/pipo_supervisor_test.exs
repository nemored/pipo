defmodule PipoSupervisor.RouterTest do
  use ExUnit.Case, async: false

  defmodule TestWorker do
    use GenServer

    def start_link(test_pid, behavior \\ :ok) do
      GenServer.start_link(__MODULE__, {test_pid, behavior})
    end

    def init({test_pid, behavior}), do: {:ok, %{test_pid: test_pid, behavior: behavior}}

    def handle_call({:deliver, frame}, _from, %{behavior: :ok} = state) do
      send(state.test_pid, {:delivered, self(), frame})
      {:reply, :ok, state}
    end

    def handle_call({:deliver, _frame}, _from, %{behavior: :sleep} = state) do
      Process.sleep(100)
      {:reply, :ok, state}
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

  test "times out deliveries, marks degraded and notifies at threshold" do
    {:ok, router} =
      start_supervised(
        {PipoSupervisor.Router,
         name: :router_test_2,
         channel_mapping: %{slow: ["alpha"], source: ["alpha"]},
         call_timeout_ms: 10,
         drop_threshold: 1,
         notify_pid: self()}
      )

    {:ok, slow_worker} = TestWorker.start_link(self(), :sleep)
    {:ok, source_worker} = TestWorker.start_link(self())

    assert :ok = PipoSupervisor.Router.register_worker(router, :slow, slow_worker)
    assert :ok = PipoSupervisor.Router.register_worker(router, :source, source_worker)

    PipoSupervisor.Router.publish(router, :source, "alpha", %{})

    assert_receive {:worker_degraded, :slow, 1, :timeout}, 500
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
end
