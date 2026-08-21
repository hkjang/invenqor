import {access, cp, mkdir, rm} from "node:fs/promises";

const buildDirectory = new URL("../dist/", import.meta.url);
const embeddedDirectory = new URL(
  "../../server/internal/webui/dist/",
  import.meta.url,
);

// Validate the complete Vite output before replacing the embedded copy. A
// failed or interrupted frontend build must never erase the Server's UI.
await access(new URL("index.html", buildDirectory));
await rm(embeddedDirectory, {recursive: true, force: true});
await mkdir(embeddedDirectory, {recursive: true});
await cp(buildDirectory, embeddedDirectory, {recursive: true});

console.log("Synchronized the React build into the Server embedded UI.");
