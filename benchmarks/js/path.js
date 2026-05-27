const path = require("node:path").posix;

function pathScore(n) {
  let i = 0;
  let acc = 0;
  const text = "examples//agents/./string_agent.ax";
  while (i < n) {
    const normalized = path.normalize(text.replaceAll("\\", "/"));
    acc += path.basename(normalized).length;
    acc += path.isAbsolute(text) ? 1 : 2;
    i += 1;
  }
  return acc % 251;
}

process.exitCode = pathScore(1000000);
