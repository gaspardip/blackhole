await expect.poll(() => count(), { timeout: 5_000 }).toBe(1);
