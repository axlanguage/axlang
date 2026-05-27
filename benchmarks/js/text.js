function textScore(n) {
  let i = 0;
  let acc = 0;
  const text = process.env.AX_TEXT || "agent-native-compiler-runtime-pack";
  while (i < n) {
    if (text.includes("runtime")) {
      acc += text.length;
    }
    if (text.startsWith("agent")) {
      acc += 1;
    }
    if (text.endsWith("pack")) {
      acc += 2;
    }
    i += 1;
  }
  return acc % 251;
}

process.exitCode = textScore(5000000);
