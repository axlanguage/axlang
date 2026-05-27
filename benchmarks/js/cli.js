function hasArg(args, name) {
  const prefix = `${name}=`;
  return args.slice(2).some((arg) => arg === name || arg.startsWith(prefix));
}

function valueArg(args, name) {
  const prefix = `${name}=`;
  for (let i = 2; i < args.length; i += 1) {
    const arg = args[i];
    if (arg.startsWith(prefix)) {
      return arg.slice(prefix.length);
    }
    if (arg === name && i + 1 < args.length) {
      return args[i + 1];
    }
  }
  return "";
}

function cliScore(args, n) {
  let i = 0;
  let acc = 0;
  while (i < n) {
    if (hasArg(args, "--input")) {
      acc += valueArg(args, "--input").length;
    }
    if (hasArg(args, "--mode")) {
      acc += valueArg(args, "--mode").length;
    }
    i += 1;
  }
  return acc % 251;
}

process.exitCode = cliScore(process.argv, 100000);
