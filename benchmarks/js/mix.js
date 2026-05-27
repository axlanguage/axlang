function mix(n) {
  let i = 0;
  let acc = 0;
  while (i < n) {
    acc = (acc + (i * 31)) % 1000003;
    i += 1;
  }
  return acc;
}

process.exitCode = mix(10000000);
