function spreadResource(flag: boolean) {
  using empty = {};
  return { ...(flag ? empty : { ready: true }) };
}
