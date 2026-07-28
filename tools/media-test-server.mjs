import { createReadStream, statSync } from "node:fs";
import { createServer } from "node:http";
import { resolve } from "node:path";

const root = resolve(new URL("../tests/fixtures", import.meta.url).pathname);
const port = Number(process.argv[2] ?? 0);
const types = {
  ".m3u8": "application/vnd.apple.mpegurl",
  ".mpd": "application/dash+xml",
  ".ts": "video/mp2t",
  ".m4s": "video/iso.segment",
};
const server = createServer((request, response) => {
  const relative = request.url?.replace(/^\/+/, "") ?? "";
  if (!relative || relative.includes("..") || relative.includes("\\")) {
    response.writeHead(404).end();
    return;
  }
  const file = resolve(root, relative);
  try {
    const size = statSync(file).size;
    const extension = Object.keys(types).find((suffix) => file.endsWith(suffix));
    response.writeHead(200, { "content-type": extension ? types[extension] : "application/octet-stream", "content-length": size });
    createReadStream(file).pipe(response);
  } catch {
    response.writeHead(404).end();
  }
});
server.listen(port, "127.0.0.1", () => {
  const address = server.address();
  if (address && typeof address !== "string") process.stderr.write(`media-test-server http://127.0.0.1:${address.port}\n`);
});
