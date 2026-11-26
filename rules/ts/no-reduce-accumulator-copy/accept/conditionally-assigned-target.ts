items.reduce(acc => {
  const holder = { target: shared };
  if (ready) holder.target = {};
  return Object.assign(holder.target, acc);
}, {});

items.reduce(acc => {
  const holder = { target: shared };
  holder.target = {};
  if (ready) holder.target = shared;
  return Object.assign(holder.target, acc);
}, {});
