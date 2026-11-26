items.reduce(acc => {
  const holder = { target: {} };
  return ((_target) => Object.assign(holder.target, acc))(holder.target = shared);
}, {});

items.reduce(acc => {
  const holder = { target: {} };
  return (() => Object.assign(holder.target, acc)).bind(null, holder.target = shared)();
}, {});
