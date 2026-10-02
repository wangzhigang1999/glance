import { readFileSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
const dir = new URL("../dist/", import.meta.url);
let total = 0;
for (const name of ["index.html", "app.js", "app.css", "favicon.svg"]) {
  const bytes = readFileSync(new URL(name, dir));
  const gzip = gzipSync(bytes, { level: 9 });
  writeFileSync(new URL(name + ".gz", dir), gzip);
  total += gzip.length;
  console.log(`${name}: ${bytes.length} bytes → ${gzip.length} bytes gzip`);
}
if (total > 240 * 1024)
  throw Error("Embedded UI exceeds 240 KiB compressed budget");
// Only compressed assets are embedded; all generated files stay outside version control.
console.log(`Embedded UI total: ${total} bytes`);
