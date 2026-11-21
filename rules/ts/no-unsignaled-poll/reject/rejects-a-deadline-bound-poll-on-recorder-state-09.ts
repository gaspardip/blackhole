await expect.poll(() => reload.mock.settledResults[0]?.type).toBe('rejected');
