import sys


def has_arg(args, name):
    prefix = name + "="
    return any(arg == name or arg.startswith(prefix) for arg in args[1:])


def value_arg(args, name):
    prefix = name + "="
    for idx, arg in enumerate(args[1:], start=1):
        if arg.startswith(prefix):
            return arg[len(prefix):]
        if arg == name and idx + 1 < len(args):
            return args[idx + 1]
    return ""


def cli_score(args, n):
    i = 0
    acc = 0
    while i < n:
        if has_arg(args, "--input"):
            acc += len(value_arg(args, "--input"))
        if has_arg(args, "--mode"):
            acc += len(value_arg(args, "--mode"))
        i += 1
    return acc % 251


raise SystemExit(cli_score(sys.argv, 100_000))
