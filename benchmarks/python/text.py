import os


def text_score(n):
    i = 0
    acc = 0
    text = os.environ.get("AX_TEXT") or "agent-native-compiler-runtime-pack"
    while i < n:
        if "runtime" in text:
            acc += len(text)
        if text.startswith("agent"):
            acc += 1
        if text.endswith("pack"):
            acc += 2
        i += 1
    return acc % 251


raise SystemExit(text_score(5_000_000))
