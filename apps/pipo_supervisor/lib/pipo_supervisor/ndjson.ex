defmodule PipoSupervisor.NDJSON do
  @moduledoc false

  def decode_line(line) when is_binary(line) do
    cond do
      String.contains?(line, "\"event\":\"ready\"") or
          String.contains?(line, "\"event\": \"ready\"") ->
        {:ok, %{"event" => "ready"}}

      true ->
        with [_, bus] <- Regex.run(~r/"bus"\s*:\s*"([^"]+)"/, line),
             [_, payload_raw] <-
               Regex.run(
                 ~r/"payload"\s*:\s*(\{.*\}|\[.*\]|".*"|true|false|null|-?\d+(?:\.\d+)?)/,
                 line
               ) do
          {:ok, %{"event" => "publish", "bus" => bus, "payload" => decode_payload(payload_raw)}}
        else
          _ -> {:error, :invalid_json}
        end
    end
  end

  def encode_frame(map) when is_map(map) do
    "{" <>
      (map
       |> Enum.map(fn {k, v} -> "\"#{k}\":" <> encode_value(v) end)
       |> Enum.join(",")) <> "}"
  end

  defp decode_payload("{}"), do: %{}
  defp decode_payload("[]"), do: []
  defp decode_payload("true"), do: true
  defp decode_payload("false"), do: false
  defp decode_payload("null"), do: nil

  defp decode_payload(payload) do
    case Integer.parse(payload) do
      {int, ""} -> int
      _ -> String.trim(payload, "\"")
    end
  end

  defp encode_value(value) when is_binary(value), do: "\"#{value}\""
  defp encode_value(value) when is_integer(value), do: Integer.to_string(value)
  defp encode_value(value) when is_boolean(value), do: if(value, do: "true", else: "false")
  defp encode_value(nil), do: "null"
  defp encode_value(value) when is_map(value), do: encode_frame(value)

  defp encode_value(value) when is_list(value),
    do: "[" <> Enum.map_join(value, ",", &encode_value/1) <> "]"
end
