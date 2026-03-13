defmodule PipoSupervisorTest do
  use ExUnit.Case, async: true

  test "returns status" do
    assert :ok == PipoSupervisor.status()
  end
end
