defmodule PipoSupervisor.Router do
  @moduledoc """
  Routes publish events between port workers based on bus subscriptions.
  """

  use GenServer
  require Logger

  @default_call_timeout_ms 1_000
  @default_drop_threshold 10

  defstruct routing_table: %{},
            worker_pids: %{},
            worker_health: %{},
            next_pipo_id: 1,
            call_timeout_ms: @default_call_timeout_ms,
            drop_threshold: @default_drop_threshold,
            notify_pid: nil

  def start_link(opts \\ []) do
    name = Keyword.get(opts, :name, __MODULE__)
    GenServer.start_link(__MODULE__, opts, name: name)
  end

  def register_worker(router \\ __MODULE__, worker_id, pid) do
    GenServer.call(router, {:register_worker, worker_id, pid})
  end

  def publish(router \\ __MODULE__, origin_worker_id, bus, payload) do
    GenServer.cast(router, {:publish, origin_worker_id, bus, payload})
  end

  @impl true
  def init(opts) do
    config = Application.get_env(:pipo_supervisor, :router, [])
    merged = Keyword.merge(config, opts)

    state = %__MODULE__{
      routing_table: build_routing_table(Keyword.get(merged, :channel_mapping, %{})),
      call_timeout_ms: Keyword.get(merged, :call_timeout_ms, @default_call_timeout_ms),
      drop_threshold: Keyword.get(merged, :drop_threshold, @default_drop_threshold),
      notify_pid: Keyword.get(merged, :notify_pid)
    }

    {:ok, state}
  end

  @impl true
  def handle_call({:register_worker, worker_id, pid}, _from, state) do
    monitor_ref = Process.monitor(pid)

    worker_pids = Map.put(state.worker_pids, worker_id, {pid, monitor_ref})
    worker_health = Map.put_new(state.worker_health, worker_id, %{degraded?: false, drops: 0})

    {:reply, :ok, %{state | worker_pids: worker_pids, worker_health: worker_health}}
  end

  def handle_call({:deliver, _frame}, _from, state) do
    {:reply, :ok, state}
  end

  @impl true
  def handle_cast({:publish, origin_worker_id, bus, payload}, state) do
    recipient_ids =
      state.routing_table
      |> Map.get(bus, MapSet.new())
      |> MapSet.delete(origin_worker_id)
      |> MapSet.to_list()

    pipo_id = state.next_pipo_id

    frame = %{"event" => "deliver", "pipo_id" => pipo_id, "bus" => bus, "payload" => payload}

    next_state =
      Enum.reduce(recipient_ids, %{state | next_pipo_id: pipo_id + 1}, fn worker_id, acc ->
        deliver_to_worker(acc, worker_id, frame)
      end)

    {:noreply, next_state}
  end

  @impl true
  def handle_info({:DOWN, ref, :process, _pid, _reason}, state) do
    worker_pids =
      state.worker_pids
      |> Enum.reject(fn {_worker_id, {_pid, mref}} -> mref == ref end)
      |> Map.new()

    {:noreply, %{state | worker_pids: worker_pids}}
  end

  defp deliver_to_worker(state, worker_id, frame) do
    case Map.get(state.worker_pids, worker_id) do
      {pid, _ref} ->
        do_deliver(state, worker_id, pid, frame)

      nil ->
        state
    end
  end

  defp do_deliver(state, worker_id, pid, frame) do
    try do
      case GenServer.call(pid, {:deliver, frame}, state.call_timeout_ms) do
        :ok ->
          put_health(state, worker_id, %{degraded?: false, drops: 0})

        _ ->
          increment_drop(state, worker_id, :unexpected_response)
      end
    catch
      :exit, {:timeout, _} ->
        Logger.warning("router timed out delivering to #{inspect(worker_id)}")
        increment_drop(state, worker_id, :timeout)

      :exit, reason ->
        Logger.warning("router failed delivering to #{inspect(worker_id)}: #{inspect(reason)}")
        increment_drop(state, worker_id, :exit)
    end
  end

  defp put_health(state, worker_id, health) do
    %{state | worker_health: Map.put(state.worker_health, worker_id, health)}
  end

  defp increment_drop(state, worker_id, reason) do
    health = Map.get(state.worker_health, worker_id, %{degraded?: false, drops: 0})
    drops = health.drops + 1
    degraded = %{health | degraded?: true, drops: drops}

    maybe_notify_degraded(state.notify_pid, worker_id, drops, state.drop_threshold, reason)

    put_health(state, worker_id, degraded)
  end

  defp maybe_notify_degraded(nil, _worker_id, _drops, _threshold, _reason), do: :ok

  defp maybe_notify_degraded(pid, worker_id, drops, threshold, reason) when drops >= threshold do
    send(pid, {:worker_degraded, worker_id, drops, reason})
  end

  defp maybe_notify_degraded(_pid, _worker_id, _drops, _threshold, _reason), do: :ok

  defp build_routing_table(channel_mapping) when is_map(channel_mapping) do
    Enum.reduce(channel_mapping, %{}, fn {worker_id, busses}, acc ->
      Enum.reduce(List.wrap(busses), acc, fn bus, inner ->
        Map.update(inner, bus, MapSet.new([worker_id]), &MapSet.put(&1, worker_id))
      end)
    end)
  end
end
