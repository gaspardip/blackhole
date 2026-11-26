function run(...a: number[]) {
  return items.reduce((acc, n) => acc.concat(n), a);
}
