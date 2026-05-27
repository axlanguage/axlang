function urlScore(n) {
  let i = 0;
  let acc = 0;
  const target = "https://agent.local/tools/search?q=Ax%20language&mode=fast";
  while (i < n) {
    const parsed = new URL(target);
    const query = parsed.searchParams.get("q") ?? "";
    const encoded = encodeURIComponent(parsed.pathname);
    const decoded = decodeURIComponent(encoded);
    acc += parsed.hostname.length;
    acc += decoded.length;
    acc += query.length;
    i += 1;
  }
  return acc % 251;
}

process.exitCode = urlScore(100000);
