from urllib.parse import parse_qs, quote, unquote, urlparse


def url_score(n):
    i = 0
    acc = 0
    target = "https://agent.local/tools/search?q=Ax%20language&mode=fast"
    while i < n:
        parsed = urlparse(target)
        query = parse_qs(parsed.query).get("q", [""])[0]
        encoded = quote(parsed.path, safe="")
        decoded = unquote(encoded)
        acc += len(parsed.hostname or "")
        acc += len(decoded)
        acc += len(query)
        i += 1
    return acc % 251


raise SystemExit(url_score(100_000))
