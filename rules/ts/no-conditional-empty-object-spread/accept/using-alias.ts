function spreadAfterDisposal(flag: boolean) {
  Object.defineProperty(Object.prototype, Symbol.dispose, {
    configurable: true,
    value(this: { tag?: number }) {
      this.tag = 1;
    },
  });

  try {
    const empty = {};
    {
      using alias = empty;
    }
    return { ...(flag ? empty : { ready: true }) };
  } finally {
    Reflect.deleteProperty(Object.prototype, Symbol.dispose);
  }
}
