static int mix(int n) {
  int i = 0;
  int acc = 0;
  while (i < n) {
    acc = (acc + (i * 31)) % 1000003;
    i = i + 1;
  }
  return acc;
}

int main(void) {
  return mix(10000000);
}
