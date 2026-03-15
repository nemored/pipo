defmodule Mix.Tasks.Pipo.BurnIn do
  use Mix.Task

  @shortdoc "Runs long-running router/worker burn-in stability validation"

  @moduledoc """
  Runs the Pipo supervisor burn-in profile with representative router traffic.

  Example (multi-hour profile):

      mix pipo.burn_in --duration-hours 4 --sample-interval-ms 5000 --publish-interval-ms 25
  """

  @switches [
    duration_hours: :integer,
    duration_ms: :integer,
    sample_interval_ms: :integer,
    publish_interval_ms: :integer,
    call_timeout_ms: :integer,
    growth_limit_mb: :integer,
    max_stall_ms: :integer,
    min_delivery_ratio: :float
  ]

  @impl true
  def run(args) do
    Mix.Task.run("app.start")

    {opts, _rest, _invalid} = OptionParser.parse(args, strict: @switches)

    duration_ms =
      cond do
        opts[:duration_ms] -> opts[:duration_ms]
        opts[:duration_hours] -> :timer.hours(opts[:duration_hours])
        true -> :timer.hours(4)
      end

    run_opts =
      [duration_ms: duration_ms]
      |> put_opt(opts, :sample_interval_ms)
      |> put_opt(opts, :publish_interval_ms)
      |> put_opt(opts, :call_timeout_ms)
      |> put_opt_mb(opts, :growth_limit_mb, :growth_limit_bytes)
      |> put_opt(opts, :max_stall_ms)
      |> put_opt(opts, :min_delivery_ratio)

    case PipoSupervisor.BurnIn.run(run_opts) do
      {:ok, report} ->
        Mix.shell().info("burn-in passed")
        Mix.shell().info(format_report(report))

      {:error, report} ->
        Mix.shell().error("burn-in failed")
        Mix.shell().error(format_report(report))
        Mix.raise("burn-in guardrails failed")
    end
  end

  defp put_opt(run_opts, parsed, key) do
    case parsed[key] do
      nil -> run_opts
      value -> Keyword.put(run_opts, key, value)
    end
  end

  defp put_opt_mb(run_opts, parsed, mb_key, bytes_key) do
    case parsed[mb_key] do
      nil -> run_opts
      mb -> Keyword.put(run_opts, bytes_key, mb * 1024 * 1024)
    end
  end

  defp format_report(report) do
    memory_lines =
      report.memory
      |> Enum.map(fn {name, details} ->
        "  #{name}: pass=#{details.pass?} growth_bytes=#{details.growth_bytes} monotonic=#{details.monotonic_non_decreasing?}"
      end)
      |> Enum.join("\n")

    """
    pass?: #{report.pass?}
    duration_ms: #{report.duration_ms}
    counters: published=#{report.counters.published} delivered=#{report.counters.delivered}
    deadlock_livelock: #{inspect(report.guardrails.deadlock_livelock)}
    message_flow: #{inspect(report.guardrails.message_flow)}
    transient_failure_recovery: #{inspect(report.guardrails.transient_failure_recovery)}
    memory:
    #{memory_lines}
    """
  end
end
