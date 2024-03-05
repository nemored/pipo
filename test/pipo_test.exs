defmodule PipoTest do
  use ExUnit.Case
  doctest Pipo

  test "greets the world" do
    assert Pipo.hello() == :world
  end
end
