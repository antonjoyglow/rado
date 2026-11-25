// A stand-in for Rado's e-ink endpoint, for the desktop preview: GET /v1/list serves a JSON file, re-read on every
// request, so editing the file is a change on the laptop. Usage: node mock-rado.mjs <list.json> [port] [token]
import { createServer } from "node:http";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const [file, port = "7878", token = "preview-token"] = process.argv.slice(2);
if (!file) {
  console.error("usage: node mock-rado.mjs <list.json> [port] [token]");
  process.exit(2);
}

createServer((req, res) => {
  const send = (status, body, headers = {}) => {
    // Content-Length, as Rado sends it: without one Node chunks the body, which the device doesn't read.
    res.writeHead(status, { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body), Connection: "close", ...headers });
    res.end(body);
    console.log(`${new Date().toLocaleTimeString()} ${req.method} ${req.url} ${status}`);
  };
  if (req.headers.authorization !== `Bearer ${token}`) return send(401, '{"error":"unauthorized"}');
  if (req.method !== "GET" || req.url !== "/v1/list") return send(404, '{"error":"not found"}');
  const body = readFileSync(file);
  const etag = `"${createHash("sha256").update(body).digest("hex").slice(0, 16)}"`;
  if (req.headers["if-none-match"] === etag) return send(304, "", { ETag: etag });
  send(200, body, { ETag: etag });
}).listen(Number(port), () => console.log(`mock Rado on port ${port}, serving ${file}`));
