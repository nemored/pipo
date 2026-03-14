defmodule PipoSupervisor.Application do
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    opts = Application.get_env(:pipo_supervisor, :supervisor, [])

    case PipoSupervisor.Supervisor.start_link(opts) do
      {:ok, _pid} = ok ->
        maybe_require_all_ready(opts)
        ok

      other ->
        other
    end
  end

  defp maybe_require_all_ready(opts) do
    if Keyword.get(opts, :require_all_ready, false) do
      workers = Keyword.get(opts, :workers, Application.get_env(:pipo_supervisor, :workers, []))
      timeout = Keyword.get(opts, :require_all_ready_timeout_ms, 5_000)

      Enum.each(workers, fn worker_id ->
        worker = GenServer.whereis(PipoSupervisor.PortWorker.via_name(worker_id))

        case worker && GenServer.call(worker, :status, timeout) do
          :ready -> :ok
          status -> raise "worker #{inspect(worker_id)} not ready: #{inspect(status)}"
        end
      end)
    end
  end
end
