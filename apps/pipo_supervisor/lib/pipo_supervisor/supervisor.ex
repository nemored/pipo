defmodule PipoSupervisor.Supervisor do
  @moduledoc false

  use Supervisor

  def start_link(opts \\ []) do
    name = Keyword.get(opts, :name, __MODULE__)
    Supervisor.start_link(__MODULE__, opts, name: name)
  end

  @impl true
  def init(opts) do
    workers = Keyword.get(opts, :workers, Application.get_env(:pipo_supervisor, :workers, []))
    router_opts = Keyword.get(opts, :router, [])
    worker_opts = Keyword.get(opts, :port_worker, [])
    worker_overrides = Keyword.get(opts, :worker_overrides, %{})

    children = [
      {PipoSupervisor.Router, router_opts}
      | Enum.map(workers, fn worker_id ->
          worker_specific_opts = Map.get(worker_overrides, worker_id, [])

          merged_worker_opts =
            worker_opts
            |> Keyword.merge(worker_specific_opts)
            |> Keyword.put(:id, worker_id)

          %{
            id: {:port_worker, worker_id},
            start: {PipoSupervisor.PortWorker, :start_link, [merged_worker_opts]},
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
