import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(new URL("..", import.meta.url).pathname);
const bridge = JSON.parse(await readFile(resolve(root, "schemas/bridge-v1.schema.json"), "utf8"));
const diagnostics = JSON.parse(await readFile(resolve(root, "schemas/diagnostics-v1.schema.json"), "utf8"));

if (bridge.type !== "object" || bridge.properties?.v?.const !== 1 || bridge.properties?.method?.enum?.length === 0) {
  throw new Error("bridge schema must define at least one method");
}
const methods = bridge.properties.method.enum;
if (new Set(methods).size !== methods.length || !methods.includes("bridge.hello")) {
  throw new Error("bridge schema methods must be unique and include bridge.hello");
}
if (diagnostics.properties?.protocolVersion?.const !== 1) {
  throw new Error("diagnostics schema protocol version must remain v1");
}
process.stdout.write(`validated ${methods.length} bridge methods\n`);
