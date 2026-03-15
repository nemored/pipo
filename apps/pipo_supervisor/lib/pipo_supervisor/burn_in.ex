defmodule PipoSupervisor.BurnIn do
  @moduledoc """
  Burn-in profile runner for long-running stability and memory trend validation.

  It drives representative publish traffic through `PipoSupervisor.Router`, samples
  per-process memory usage for the router and workers, and evaluates guardrails:

    * no monotonic, unbounded memory growth
    * no deadlock/livelock
    * stable message flow despite transient worker failures
  """

  @default_duration_ms :timer.hours(4)
  @default_publish_interval_ms 25
  @default_sample_interval_ms 5_000
  @default_call_timeout_ms 20
  @default_growth_limit_bytes 32 * 1024 * 1024
  @default_max_stall_ms 10_000
  @default_min_delivery_ratio 0.60

  def run(opts \\ []) do
    duration_ms = Keyword.get(opts, :duration_ms, @default_duration_ms)
    publish_interval_ms = Keyword.get(opts, :publish_interval_ms, @default_publish_interval_ms)
    sample_interval_ms = Keyword.get(opts, :sample_interval_ms, @default_sample_interval_ms)
    call_timeout_ms = Keyword.get(opts, :call_timeout_ms, @default_call_timeout_ms)
    growth_limit_bytes = Keyword.get(opts, :growth_limit_bytes, @default_growth_limit_bytes)
    max_stall_ms = Keyword.get(opts, :max_stall_ms, @default_max_stall_ms)
    min_delivery_ratio = Keyword.get(opts, :min_delivery_ratio, @default_min_delivery_ratio)

    run_id = System.unique_integer([:positive])
    router_name = String.to_atom("burn_in_router_#{run_id}")

    {:ok, counter} =
      Agent.start_link(fn -> %{published: 0, delivered: 0, last_delivery_at: nil} end)

    {:ok, router} =
      PipoSupervisor.Router.start_link(
        name: router_name,
        call_timeout_ms: call_timeout_ms,
        drop_threshold: 1,
        notify_pid: self(),
        channel_mapping: %{stable_a: ["alpha"], stable_b: ["alpha"], flaky: ["alpha"]}
      )

    {:ok, stable_a} = __MODULE__.BurnInWorker.start_link(id: :stable_a, counter: counter)
    {:ok, stable_b} = __MODULE__.BurnInWorker.start_link(id: :stable_b, counter: counter)

    {:ok, flaky} =
      __MODULE__.BurnInWorker.start_link(id: :flaky, counter: counter, fail_mode: :toggle)

    :ok = PipoSupervisor.Router.register_worker(router, :stable_a, stable_a)
    :ok = PipoSupervisor.Router.register_worker(router, :stable_b, stable_b)
    :ok = PipoSupervisor.Router.register_worker(router, :flaky, flaky)

    publisher = spawn_link(fn -> publish_loop(router, counter, publish_interval_ms, 1) end)
    flapper = spawn_link(fn -> failure_loop(flaky, 600, 250) end)

    tracked = %{router: router, stable_a: stable_a, stable_b: stable_b, flaky: flaky}

    started_at = System.monotonic_time(:millisecond)

    {samples, transitions} =
      collect_samples(tracked, started_at, duration_ms, sample_interval_ms, [])

    send(publisher, :stop)
    send(flapper, :stop)

    counters = Agent.get(counter, & &1)

    report =
      build_report(
        samples,
        transitions,
        counters,
        started_at,
        growth_limit_bytes,
        max_stall_ms,
        min_delivery_ratio
      )

    Process.exit(router, :normal)
    Process.exit(stable_a, :normal)
    Process.exit(stable_b, :normal)
    Process.exit(flaky, :normal)
    Agent.stop(counter)

    if report.pass?, do: {:ok, report}, else: {:error, report}
  end

  defp collect_samples(tracked, started_at, duration_ms, sample_interval_ms, transitions) do
    now = System.monotonic_time(:millisecond)
    elapsed = now - started_at

    sample =
      tracked
      |> Enum.map(fn {name, pid} ->
        mem = if Process.alive?(pid), do: process_memory(pid), else: 0
        {name, %{pid: pid, memory_bytes: mem}}
      end)
      |> Map.new()
      |> Map.put(:at_ms, elapsed)

    transitions = drain_transitions(transitions)

    if elapsed >= duration_ms do
      {[sample], transitions}
    else
      Process.sleep(sample_interval_ms)

      {tail, transitions} =
        collect_samples(tracked, started_at, duration_ms, sample_interval_ms, transitions)

      {[sample | tail], transitions}
    end
  end

  defp drain_transitions(transitions) do
    receive do
      {:worker_degraded, worker_id, drops, reason} ->
        stamp = System.monotonic_time(:millisecond)
        drain_transitions([{:degraded, worker_id, drops, reason, stamp} | transitions])

      {:worker_restored, worker_id} ->
        stamp = System.monotonic_time(:millisecond)
        drain_transitions([{:restored, worker_id, stamp} | transitions])
    after
      0 -> Enum.reverse(transitions)
    end
  end

  defp process_memory(pid) do
    case Process.info(pid, :memory) do
      {:memory, bytes} -> bytes
      _ -> 0
    end
  end

  defp publish_loop(router, counter, interval_ms, seq) do
    receive do
      :stop ->
        :ok
    after
      interval_ms ->
        payload = %{"seq" => seq, "at_ms" => System.monotonic_time(:millisecond)}
        PipoSupervisor.Router.publish(router, :source, "alpha", payload)
        Agent.update(counter, &Map.update!(&1, :published, fn x -> x + 1 end))
        publish_loop(router, counter, interval_ms, seq + 1)
    end
  end

  defp failure_loop(worker_pid, healthy_ms, failing_ms) do
    receive do
      :stop ->
        :ok
    after
      healthy_ms ->
        __MODULE__.BurnInWorker.set_failure(worker_pid, true)
        Process.sleep(failing_ms)
        __MODULE__.BurnInWorker.set_failure(worker_pid, false)
        failure_loop(worker_pid, healthy_ms, failing_ms)
    end
  end

  defp build_report(
         samples,
         transitions,
         counters,
         started_at,
         growth_limit_bytes,
         max_stall_ms,
         min_delivery_ratio
       ) do
    ended_at = System.monotonic_time(:millisecond)
    duration_ms = ended_at - started_at

    memory =
      [:router, :stable_a, :stable_b, :flaky]
      |> Enum.map(fn name -> {name, memory_profile(samples, name, growth_limit_bytes)} end)
      |> Map.new()

    deadlock = deadlock_guardrail(counters, duration_ms, max_stall_ms)
    flow = flow_guardrail(counters, min_delivery_ratio)
    recovery = recovery_guardrail(transitions)

    pass? =
      Enum.all?([deadlock.pass?, flow.pass?, recovery.pass?]) and
        Enum.all?(Map.values(memory), & &1.pass?)

    %{
      pass?: pass?,
      duration_ms: duration_ms,
      counters: counters,
      transitions: transitions,
      memory: memory,
      guardrails: %{
        memory: memory,
        deadlock_livelock: deadlock,
        message_flow: flow,
        transient_failure_recovery: recovery
      }
    }
  end

  defp memory_profile(samples, name, growth_limit_bytes) do
    series = Enum.map(samples, &get_in(&1, [name, :memory_bytes]))
    start_mem = List.first(series) || 0
    end_mem = List.last(series) || 0
    growth = end_mem - start_mem
    monotonic? = monotonic_non_decreasing?(series)
    bounded? = growth <= growth_limit_bytes
    pass? = not monotonic? or bounded?

    %{
      pass?: pass?,
      start_bytes: start_mem,
      end_bytes: end_mem,
      growth_bytes: growth,
      monotonic_non_decreasing?: monotonic?,
      bounded_growth?: bounded?
    }
  end

  defp monotonic_non_decreasing?([_single]), do: true
  defp monotonic_non_decreasing?([]), do: true

  defp monotonic_non_decreasing?(series) do
    series
    |> Enum.chunk_every(2, 1, :discard)
    |> Enum.all?(fn [a, b] -> b >= a end)
  end

  defp deadlock_guardrail(counters, duration_ms, max_stall_ms) do
    stalled_ms =
      case counters.last_delivery_at do
        nil -> duration_ms
        ts -> System.monotonic_time(:millisecond) - ts
      end

    pass? = counters.published > 0 and counters.delivered > 0 and stalled_ms <= max_stall_ms

    %{pass?: pass?, stalled_ms: stalled_ms, max_stall_ms: max_stall_ms}
  end

  defp flow_guardrail(counters, min_delivery_ratio) do
    ratio =
      if counters.published == 0,
        do: 0.0,
        else: counters.delivered / max(counters.published * 3, 1)

    pass? = ratio >= min_delivery_ratio

    %{pass?: pass?, delivery_ratio: ratio, min_delivery_ratio: min_delivery_ratio}
  end

  defp recovery_guardrail(transitions) do
    degraded = Enum.count(transitions, &(elem(&1, 0) == :degraded))
    restored = Enum.count(transitions, &(elem(&1, 0) == :restored))
    pass? = degraded > 0 and restored > 0
    %{pass?: pass?, degraded_events: degraded, restored_events: restored}
  end

  defmodule BurnInWorker do
    use GenServer

    def start_link(opts) do
      GenServer.start_link(__MODULE__, opts)
    end

    def set_failure(pid, enabled) do
      GenServer.cast(pid, {:set_failure, enabled})
    end

    @impl true
    def init(opts) do
      {:ok,
       %{
         id: Keyword.fetch!(opts, :id),
         counter: Keyword.fetch!(opts, :counter),
         fail_mode: Keyword.get(opts, :fail_mode, :never),
         failing?: false
       }}
    end

    @impl true
    def handle_cast({:set_failure, enabled}, state), do: {:noreply, %{state | failing?: enabled}}

    @impl true
    def handle_call({:deliver, _frame}, _from, %{fail_mode: :toggle, failing?: true} = state) do
      {:reply, {:error, :transient_failure}, state}
    end

    def handle_call({:deliver, _frame}, _from, state) do
      now = System.monotonic_time(:millisecond)

      Agent.update(state.counter, fn counters ->
        counters
        |> Map.update!(:delivered, &(&1 + 1))
        |> Map.put(:last_delivery_at, now)
      end)

      {:reply, :ok, state}
    end
  end
end
