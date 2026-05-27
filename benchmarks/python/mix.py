def mix(n):
    i = 0
    acc = 0
    while i < n:
        acc = (acc + (i * 31)) % 1_000_003
        i += 1
    return acc


raise SystemExit(mix(10_000_000))
