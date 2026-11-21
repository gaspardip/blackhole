await expect
  .poll(() => mutate.mock.calls.length)
  .toBe(1);
