items.reduce(acc => {
  const holder = { target: shared };
  holder.target = {};
  return Object.assign(holder.target, acc);
}, {});

items.reduce(acc => {
  const holder = { target: shared };
  if (ready) {
    holder.target = {};
    return Object.assign(holder.target, acc);
  }
  return acc;
}, {});
