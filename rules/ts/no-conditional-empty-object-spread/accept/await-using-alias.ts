async function spreadAfterDisposal(flag: boolean) {
  Object.defineProperty(Object.prototype, Symbol.asyncDispose, {
    configurable: true,
    async value(this: { tag?: number }) {
      this.tag = 1;
    },
  });

  try {
    const empty = {};
    {
      await using alias = empty;
    }
    return { ...(flag ? empty : { ready: true }) };
  } finally {
    Reflect.deleteProperty(Object.prototype, Symbol.asyncDispose);
  }
}
