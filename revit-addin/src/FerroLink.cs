// Ferro Link client: talks to the Ferro app open on this computer.
// Protocol (see crates/ferro-tools/src/link.rs): read ~/.ferro/link.json,
// connect to 127.0.0.1:<port>, send one JSON line, read one JSON line.

using System;
using System.IO;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Ferro.Revit
{
    internal sealed class LinkInfo
    {
        public int Port { get; init; }
        public string Token { get; init; } = "";
        public string Project { get; init; } = "";
    }

    internal sealed class LinkException : Exception
    {
        public LinkException(string message) : base(message) { }
    }

    internal static class FerroLink
    {
        public static string FerroDir()
        {
            var env = Environment.GetEnvironmentVariable("FERRO_HOME");
            if (!string.IsNullOrEmpty(env)) return env;
            return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".ferro");
        }

        /// <summary>The open Ferro app, or null when none is running.</summary>
        public static LinkInfo? Find()
        {
            var path = Path.Combine(FerroDir(), "link.json");
            if (!File.Exists(path)) return null;
            try
            {
                var j = JsonNode.Parse(File.ReadAllText(path));
                if (j == null) return null;
                return new LinkInfo
                {
                    Port = j["port"]?.GetValue<int>() ?? 0,
                    Token = j["token"]?.GetValue<string>() ?? "",
                    Project = j["project"]?.GetValue<string>() ?? "",
                };
            }
            catch (Exception) { return null; }
        }

        /// <summary>Call a Ferro tool. Throws LinkException with Ferro's message on failure.</summary>
        public static JsonNode Call(LinkInfo link, string tool, JsonNode args, int timeoutMs = 120_000)
        {
            var req = new JsonObject
            {
                ["token"] = link.Token,
                ["tool"] = tool,
                ["args"] = args,
                ["agent"] = new JsonObject { ["name"] = "Revit", ["version"] = typeof(FerroLink).Assembly.GetName().Version?.ToString() ?? "" },
            };
            using var client = new TcpClient();
            var connect = client.ConnectAsync("127.0.0.1", link.Port);
            if (!connect.Wait(1000)) throw new LinkException("Ferro is not answering. Is the Ferro app open?");
            client.ReceiveTimeout = timeoutMs;
            client.SendTimeout = timeoutMs;
            using var stream = client.GetStream();
            var line = Encoding.UTF8.GetBytes(req.ToJsonString() + "\n");
            stream.Write(line, 0, line.Length);
            using var reader = new StreamReader(stream, new UTF8Encoding(false));
            var answer = reader.ReadLine() ?? throw new LinkException("Ferro closed the connection.");
            var resp = JsonNode.Parse(answer) ?? throw new LinkException("Ferro sent an empty answer.");
            var body = resp["body"] ?? new JsonObject();
            if (resp["ok"]?.GetValue<bool>() != true)
            {
                var msg = body["error"]?.GetValue<string>() ?? "Ferro refused the request.";
                var first = body["failures"]?[0]?["message"]?.GetValue<string>();
                throw new LinkException(first == null ? msg : $"{msg}: {first}");
            }
            return body;
        }

        public static readonly JsonSerializerOptions Json = new() { WriteIndented = false };
    }
}
