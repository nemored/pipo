defmodule PipoSupervisor.Router do
  @moduledoc """
  Routes publish events between port workers based on bus subscriptions.
  """

  use GenServer
  require Logger

  @default_call_timeout_ms 1_000
  @default_drop_threshold 10

  defstruct routing_table: %{},
            worker_buses: %{},
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

  def unregister_worker(router \\ __MODULE__, worker_id) do
    GenServer.call(router, {:unregister_worker, worker_id})
  end

  def add_subscription(router \\ __MODULE__, worker_id, bus) do
    GenServer.call(router, {:add_subscription, worker_id, bus})
  end

  def remove_subscription(router \\ __MODULE__, worker_id, bus) do
    GenServer.call(router, {:remove_subscription, worker_id, bus})
  end

  def publish(router \\ __MODULE__, origin_worker_id, bus, payload) do
    GenServer.cast(router, {:publish, origin_worker_id, bus, payload})
  end

  @impl true
  def init(opts) do
    config = Application.get_env(:pipo_supervisor, :router, [])
    merged = Keyword.merge(config, opts)

    {routing_table, worker_buses} =
      build_routing_tables(Keyword.get(merged, :channel_mapping, %{}))

    state = %__MODULE__{
      routing_table: routing_table,
      worker_buses: worker_buses,
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

  def handle_call({:unregister_worker, worker_id}, _from, state) do
    {:reply, :ok, unregister_worker_from_state(state, worker_id)}
  end

  def handle_call({:add_subscription, worker_id, bus}, _from, state) do
    {:reply, :ok, add_subscription_to_state(state, worker_id, bus)}
  end

  def handle_call({:remove_subscription, worker_id, bus}, _from, state) do
    {:reply, :ok, remove_subscription_from_state(state, worker_id, bus)}
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
    down_worker_id =
      state.worker_pids
      |> Enum.find_value(fn {worker_id, {_pid, mref}} -> if mref == ref, do: worker_id end)

    next_state =
      case down_worker_id do
        nil -> state
        worker_id -> unregister_worker_from_state(state, worker_id)
      end

    {:noreply, next_state}
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
          restore_worker(state, worker_id)

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

    if not health.degraded? do
      Logger.warning(
        "router worker_degraded worker_id=#{inspect(worker_id)} drops=#{drops} reason=#{inspect(reason)}"
      )
    end

    maybe_notify_degraded(state.notify_pid, worker_id, drops, state.drop_threshold, reason)

    put_health(state, worker_id, degraded)
  end

  defp restore_worker(state, worker_id) do
    health = Map.get(state.worker_health, worker_id, %{degraded?: false, drops: 0})

    if health.degraded? do
      Logger.info("router worker_restored worker_id=#{inspect(worker_id)} drops=#{health.drops}")
      maybe_notify_restored(state.notify_pid, worker_id)
    end

    put_health(state, worker_id, %{degraded?: false, drops: 0})
  end

  defp maybe_notify_degraded(nil, _worker_id, _drops, _threshold, _reason), do: :ok

  defp maybe_notify_degraded(pid, worker_id, drops, threshold, reason) when drops >= threshold do
    send(pid, {:worker_degraded, worker_id, drops, reason})
  end

  defp maybe_notify_degraded(_pid, _worker_id, _drops, _threshold, _reason), do: :ok

  defp maybe_notify_restored(nil, _worker_id), do: :ok

  defp maybe_notify_restored(pid, worker_id) do
    send(pid, {:worker_restored, worker_id})
  end

  defp build_routing_tables(channel_mapping) when is_map(channel_mapping) do
    Enum.reduce(channel_mapping, {%{}, %{}}, fn {worker_id, busses},
                                                {routing_table, worker_buses} ->
      Enum.reduce(List.wrap(busses), {routing_table, worker_buses}, fn bus,
                                                                       {inner_routing,
                                                                        inner_worker_buses} ->
        next_routing =
          Map.update(inner_routing, bus, MapSet.new([worker_id]), &MapSet.put(&1, worker_id))

        next_worker_buses =
          Map.update(inner_worker_buses, worker_id, MapSet.new([bus]), &MapSet.put(&1, bus))

        {next_routing, next_worker_buses}
      end)
    end)
  end

  defp unregister_worker_from_state(state, worker_id) do
    worker_pids = Map.delete(state.worker_pids, worker_id)
    worker_health = Map.delete(state.worker_health, worker_id)

    {routing_table, worker_buses} =
      case Map.get(state.worker_buses, worker_id) do
        nil ->
          {state.routing_table, state.worker_buses}

        busses ->
          remove_worker_from_buses(state.routing_table, state.worker_buses, worker_id, busses)
      end

    %{
      state
      | worker_pids: worker_pids,
        worker_health: worker_health,
        routing_table: routing_table,
        worker_buses: worker_buses
    }
  end

  defp add_subscription_to_state(state, worker_id, bus) do
    routing_table =
      Map.update(state.routing_table, bus, MapSet.new([worker_id]), &MapSet.put(&1, worker_id))

    worker_buses =
      Map.update(state.worker_buses, worker_id, MapSet.new([bus]), &MapSet.put(&1, bus))

    %{state | routing_table: routing_table, worker_buses: worker_buses}
  end

  defp remove_subscription_from_state(state, worker_id, bus) do
    routing_table =
      case Map.get(state.routing_table, bus, MapSet.new()) |> MapSet.delete(worker_id) do
        set when map_size(set) == 0 -> Map.delete(state.routing_table, bus)
        set -> Map.put(state.routing_table, bus, set)
      end

    worker_buses =
      case Map.get(state.worker_buses, worker_id, MapSet.new()) |> MapSet.delete(bus) do
        set when map_size(set) == 0 -> Map.delete(state.worker_buses, worker_id)
        set -> Map.put(state.worker_buses, worker_id, set)
      end

    %{state | routing_table: routing_table, worker_buses: worker_buses}
  end

  defp remove_worker_from_buses(routing_table, worker_buses, worker_id, busses) do
    next_routing =
      Enum.reduce(busses, routing_table, fn bus, acc ->
        case Map.get(acc, bus, MapSet.new()) |> MapSet.delete(worker_id) do
          set when map_size(set) == 0 -> Map.delete(acc, bus)
          set -> Map.put(acc, bus, set)
        end
      end)

    {next_routing, Map.delete(worker_buses, worker_id)}
  end
end
