import posixpath


def path_score(n):
    i = 0
    acc = 0
    text = "examples//agents/./string_agent.ax"
    while i < n:
        normalized = posixpath.normpath(text.replace("\\", "/"))
        acc += len(posixpath.basename(normalized))
        acc += 1 if posixpath.isabs(text) else 2
        i += 1
    return acc % 251


raise SystemExit(path_score(1_000_000))
