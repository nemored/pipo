defmodule PipoSupervisor.Supervisor do
  @moduledoc false

  use Supervisor

  def start_link(opts \\ []) do
    Supervisor.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @impl true
  def init(opts) do
    workers = Keyword.get(opts, :workers, Application.get_env(:pipo_supervisor, :workers, []))
    router_opts = Keyword.get(opts, :router, [])

    children = [
      {PipoSupervisor.Router, router_opts}
      | Enum.map(workers, fn worker_id ->
          %{
            id: {:port_worker, worker_id},
            start: {PipoSupervisor.PortWorker, :start_link, [[id: worker_id]]},
            restart: :permanent,
            shutdown: 2_000,
            type: :worker
          }
        end)
    ]

    max_restarts =
      Keyword.get(opts, :max_restarts, Application.get_env(:pipo_supervisor, :max_restarts, 3))

    max_seconds =
      Keyword.get(opts, :max_seconds, Application.get_env(:pipo_supervisor, :max_seconds, 10))

    Supervisor.init(children,
      strategy: :one_for_one,
      max_restarts: max_restarts,
      max_seconds: max_seconds
    )
  end
end
