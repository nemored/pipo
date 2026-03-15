defmodule PipoSupervisor.BurnInTest do
  use ExUnit.Case, async: false

  test "short burn-in run produces passing guardrails and process memory series" do
    assert {:ok, report} =
             PipoSupervisor.BurnIn.run(
               duration_ms: 1_200,
               publish_interval_ms: 10,
               sample_interval_ms: 100,
               call_timeout_ms: 15,
               growth_limit_bytes: 16 * 1024 * 1024,
               max_stall_ms: 500,
               min_delivery_ratio: 0.4
             )

    assert report.pass?
    assert report.guardrails.deadlock_livelock.pass?
    assert report.guardrails.message_flow.pass?
    assert report.guardrails.transient_failure_recovery.pass?

    assert Map.has_key?(report.memory, :router)
    assert Map.has_key?(report.memory, :stable_a)
    assert Map.has_key?(report.memory, :stable_b)
    assert Map.has_key?(report.memory, :flaky)

    assert Enum.all?(Map.values(report.memory), &is_boolean(&1.pass?))
    assert report.counters.published > 0
    assert report.counters.delivered > 0
  end

  test "fails when strict delivery ratio guardrail is not achievable" do
    assert {:error, report} =
             PipoSupervisor.BurnIn.run(
               duration_ms: 700,
               publish_interval_ms: 10,
               sample_interval_ms: 100,
               call_timeout_ms: 10,
               min_delivery_ratio: 0.98
             )

    refute report.pass?
    refute report.guardrails.message_flow.pass?
  end
end
