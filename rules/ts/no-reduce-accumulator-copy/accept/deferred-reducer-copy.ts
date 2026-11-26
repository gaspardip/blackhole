[1].reduce((acc, n) => {
  queueMicrotask(() => consume([...acc, n]));
  return acc;
}, []);
