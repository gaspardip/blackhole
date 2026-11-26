items.reduce(acc => {
  const holder = { target: {} };
  (holder.target satisfies object) = shared;
  return Object.assign(holder.target, acc);
}, {});

items.reduce(acc => {
  const holder = { target: {} };
  (holder.target as object) = shared;
  return Object.assign(holder.target, acc);
}, {});
