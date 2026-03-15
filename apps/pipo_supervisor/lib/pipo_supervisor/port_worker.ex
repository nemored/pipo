defmodule PipoSupervisor.PortWorker do
  @moduledoc """
  Bridges an external `pipo-transport` process and the Router.
  """

  use GenServer
  require Logger

  @default_ready_timeout_ms 5_000
  @default_shutdown_timeout_ms 1_000
  @default_backoff_ms 100
  @default_jitter_ms 50

  @restart_counter_key {__MODULE__, :restart_counts}

  defstruct id: nil,
            instance_id: nil,
            restart_count: 0,
            router: PipoSupervisor.Router,
            transport_path: nil,
            transport: nil,
            port: nil,
            status: :starting,
            ready_timeout_ms: @default_ready_timeout_ms,
            shutdown_timeout_ms: @default_shutdown_timeout_ms,
            ready_timer: nil,
            backoff_ms: @default_backoff_ms,
            jitter_ms: @default_jitter_ms

  def start_link(opts) do
    id = Keyword.fetch!(opts, :id)
    name = Keyword.get(opts, :name, via_name(id))
    GenServer.start_link(__MODULE__, opts, name: name)
  end

  def via_name(id), do: {:global, {__MODULE__, id}}

  @impl true
  def init(opts) do
    config = Application.get_env(:pipo_supervisor, :port_worker, [])
    merged = Keyword.merge(config, opts)

    id = Keyword.fetch!(merged, :id)
    restart_count = restart_count_for(id)
    transport_path = resolve_transport_path(Keyword.get(merged, :transport_path))

    state = %__MODULE__{
      id: id,
      instance_id: "#{id}-#{restart_count}",
      restart_count: restart_count,
      router: Keyword.get(merged, :router, PipoSupervisor.Router),
      transport_path: transport_path,
      transport: transport_name(transport_path),
      ready_timeout_ms: Keyword.get(merged, :ready_timeout_ms, @default_ready_timeout_ms),
      shutdown_timeout_ms:
        Keyword.get(merged, :shutdown_timeout_ms, @default_shutdown_timeout_ms),
      backoff_ms: Keyword.get(merged, :backoff_ms, @default_backoff_ms),
      jitter_ms: Keyword.get(merged, :jitter_ms, @default_jitter_ms)
    }

    backoff = state.backoff_ms + :rand.uniform(max(state.jitter_ms, 1)) - 1

    Logger.info(
      "worker init instance_id=#{state.instance_id} worker_id=#{inspect(state.id)} transport=#{state.transport} restart_count=#{state.restart_count} backoff_ms=#{backoff} jitter_ms=#{state.jitter_ms}"
    )

    Process.send_after(self(), :boot_transport, backoff)

    {:ok, state}
  end

  @impl true
  def handle_info(:boot_transport, state) do
    case launch_port(state.transport_path) do
      {:ok, port} ->
        ready_timer = Process.send_after(self(), :ready_timeout, state.ready_timeout_ms)
        maybe_register_worker(state.router, state.id)
        {:noreply, %{state | port: port, ready_timer: ready_timer, status: :starting}}

      {:error, reason} ->
        Logger.error(
          "worker boot_failed instance_id=#{state.instance_id} worker_id=#{inspect(state.id)} transport=#{state.transport} restart_count=#{state.restart_count} reason=#{inspect(reason)}"
        )

        {:stop, reason, %{state | status: :stopped}}
    end
  end

  def handle_info(:ready_timeout, state = %{status: :starting}) do
    Logger.warning(
      "worker degraded instance_id=#{state.instance_id} worker_id=#{inspect(state.id)} transport=#{state.transport} restart_count=#{state.restart_count} reason=ready_timeout"
    )

    {:noreply, %{state | status: :degraded, ready_timer: nil}}
  end

  def handle_info({port, {:data, {:eol, line}}}, %{port: port} = state) do
    next_state = handle_transport_line(String.trim(line), state)
    {:noreply, next_state}
  end

  def handle_info({port, {:data, {:noeol, line}}}, %{port: port} = state) do
    next_state = handle_transport_line(String.trim(line), state)
    {:noreply, next_state}
  end

  def handle_info({port, {:exit_status, _status}}, %{port: port} = state) do
    Logger.warning(
      "worker transport_exit instance_id=#{state.instance_id} worker_id=#{inspect(state.id)} transport=#{state.transport} restart_count=#{state.restart_count}"
    )

    {:stop, :transport_exited, %{state | status: :stopped, port: nil}}
  end

  @impl true
  def handle_call({:deliver, frame}, _from, state) do
    case state.port do
      nil ->
        {:reply, {:error, :no_port}, state}

      port ->
        payload = Jason.encode!(frame)
        Port.command(port, payload <> "\n")
        {:reply, :ok, state}
    end
  end

  def handle_call(:status, _from, state), do: {:reply, state.status, state}

  @impl true
  def terminate(_reason, state) do
    graceful_stop_port(state)
    :ok
  end

  defp handle_transport_line("", state), do: state

  defp handle_transport_line(line, state) do
    case Jason.decode(line) do
      {:ok, decoded} ->
        process_frame(decoded, state)

      {:error, _reason} ->
        Logger.warning(
          "worker invalid_frame instance_id=#{state.instance_id} worker_id=#{inspect(state.id)} transport=#{state.transport} restart_count=#{state.restart_count} line=#{inspect(line)}"
        )

        state
    end
  end

  defp process_frame(%{"type" => "ready"}, state) do
    if state.ready_timer, do: Process.cancel_timer(state.ready_timer)

    Logger.info(
      "worker ready instance_id=#{state.instance_id} worker_id=#{inspect(state.id)} transport=#{state.transport} restart_count=#{state.restart_count}"
    )

    %{state | status: :ready, ready_timer: nil}
  end

  defp process_frame(%{"type" => "message", "bus" => bus, "payload" => payload}, state) do
    case Process.whereis(state.router) do
      nil ->
        Logger.warning(
          "worker router_unavailable instance_id=#{state.instance_id} worker_id=#{inspect(state.id)} transport=#{state.transport} restart_count=#{state.restart_count}"
        )

        state

      _pid ->
        PipoSupervisor.Router.publish(state.router, state.id, bus, payload)
        state
    end
  end

  defp process_frame(_frame, state), do: state

  defp maybe_register_worker(router, id) do
    case GenServer.whereis(router) do
      nil ->
        Logger.warning("router unavailable during worker registration for #{inspect(id)}")
        :ok

      _ ->
        _ = PipoSupervisor.Router.register_worker(router, id, self())
        :ok
    end
  end

  defp graceful_stop_port(%{port: nil}), do: :ok

  defp graceful_stop_port(state) do
    shutdown_frame = Jason.encode!(%{"event" => "shutdown"}) <> "\n"
    Port.command(state.port, shutdown_frame)

    receive do
      {port, {:exit_status, _status}} when port == state.port ->
        :ok
    after
      state.shutdown_timeout_ms ->
        Port.close(state.port)
    end
  rescue
    _ -> :ok
  end

  defp launch_port(nil), do: {:error, :missing_transport_path}

  defp launch_port(path) do
    if File.exists?(path) do
      {:ok,
       Port.open({:spawn_executable, path}, [
         :binary,
         {:line, 65_536},
         :exit_status,
         :use_stdio,
         :stderr_to_stdout
       ])}
    else
      {:error, :enoent}
    end
  end

  defp restart_count_for(worker_id) do
    counts = :persistent_term.get(@restart_counter_key, %{})
    restart_count = Map.get(counts, worker_id, -1) + 1
    :persistent_term.put(@restart_counter_key, Map.put(counts, worker_id, restart_count))
    restart_count
  end

  defp transport_name(nil), do: "unknown"
  defp transport_name(path), do: Path.basename(path)

  defp resolve_transport_path(nil) do
    env_path = System.get_env("PIPO_TRANSPORT_PATH")

    release_relative =
      Application.app_dir(:pipo_supervisor, "../../../native/pipo-transport")
      |> Path.expand()

    bundled = Application.app_dir(:pipo_supervisor, "priv/pipo-transport")

    [env_path, release_relative, bundled]
    |> Enum.reject(&is_nil/1)
    |> Enum.find(&File.exists?/1)
  end

  defp resolve_transport_path(path), do: path
end
