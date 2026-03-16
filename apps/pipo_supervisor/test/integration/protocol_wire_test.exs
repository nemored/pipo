defmodule PipoSupervisor.ProtocolWireTest do
  use ExUnit.Case, async: false

  @moduletag :integration
  @port_timeout 1_500

  setup do
    {:ok, transport_bin: transport_binary_path()}
  end

  test "health round-trip", %{transport_bin: transport_bin} do
    {port, config_path} = open_transport_port(transport_bin)

    on_exit(fn ->
      shutdown_port(port)
      File.rm(config_path)
    end)

    ready_frame = recv_frame!(port)
    assert ready_frame["type"] == "ready"

    send_frame!(port, ~s({"type":"health_check"}\n))

    response = recv_frame!(port)
    assert response["type"] == "health"
    assert is_binary(response["timestamp"])
    assert String.ends_with?(response["timestamp"], "Z")
    assert response["status"] == "ok"
  end

  test "reload_config stub", %{transport_bin: transport_bin} do
    {port, config_path} = open_transport_port(transport_bin)

    on_exit(fn ->
      shutdown_port(port)
      File.rm(config_path)
    end)

    _ready = recv_frame!(port)

    send_frame!(port, ~s({"type":"reload_config"}\n))

    response = recv_frame!(port)
    assert response["type"] == "warn"
    assert response["warn_type"] == "unimplemented"
    assert Port.info(port) != nil
  end

  test "malformed frame recovery", %{transport_bin: transport_bin} do
    {port, config_path} = open_transport_port(transport_bin)

    on_exit(fn ->
      shutdown_port(port)
      File.rm(config_path)
    end)

    _ready = recv_frame!(port)

    send_frame!(port, "not-valid-json\n")

    response = recv_frame!(port)
    assert response["type"] == "warn"
    assert response["warn_type"] == "invalid_frame"
    assert Port.info(port) != nil
  end

  test "startup auth failure exits with status 10 and does not restart", %{
    transport_bin: transport_bin
  } do
    {port, config_path} = open_transport_port(transport_bin, %{"startup_failure" => "auth"})

    on_exit(fn ->
      File.rm(config_path)
    end)

    assert_receive {^port, {:exit_status, 10}}, @port_timeout
    assert Port.info(port) == nil

    refute_receive {^port, {:data, _}}, 250
    refute_receive {^port, {:exit_status, _}}, 250
  end

  test "shutdown", %{transport_bin: transport_bin} do
    {port, config_path} = open_transport_port(transport_bin)

    on_exit(fn ->
      File.rm(config_path)
    end)

    _ready = recv_frame!(port)

    send_frame!(port, ~s({"type":"shutdown"}\n))

    assert_receive {^port, {:exit_status, 0}}, @port_timeout
    assert Port.info(port) == nil
  end

  defp transport_binary_path do
    System.get_env("PIPO_TRANSPORT_BIN") ||
      fallback_transport_binary_path()
  end

  defp fallback_transport_binary_path do
    [
      Path.expand("../../../../target/debug/pipo-transport", __DIR__),
      Path.expand("../../../../native/transport_runtime/target/debug/pipo-transport", __DIR__)
    ]
    |> Enum.find(&File.exists?/1)
    |> Kernel.||(Path.expand("../../../../target/debug/pipo-transport", __DIR__))
  end

  defp open_transport_port(transport_bin, config \\ %{}) do
    config_path = write_config!(config)

    args = [
      "--transport",
      "test",
      "--config",
      config_path
    ]

    port =
      Port.open({:spawn_executable, transport_bin}, [
        :binary,
        :exit_status,
        :use_stdio,
        :stderr_to_stdout,
        :hide,
        {:line, 16_384},
        args: args
      ])

    {port, config_path}
  end

  defp write_config!(config) do
    path =
      Path.join(
        System.tmp_dir!(),
        "pipo-transport-config-#{System.unique_integer([:positive])}.json"
      )

    File.write!(path, Jason.encode!(config))
    path
  end

  defp send_frame!(port, frame) do
    assert Port.command(port, frame)
  end

  defp recv_frame!(port) do
    assert_receive {^port, {:data, {_line_type, line}}}, @port_timeout
    Jason.decode!(line)
  end

  defp shutdown_port(port) do
    if Port.info(port) != nil do
      Port.command(port, ~s({"type":"shutdown"}\n))
      assert_receive {^port, {:exit_status, 0}}, @port_timeout
    end
  end
end
