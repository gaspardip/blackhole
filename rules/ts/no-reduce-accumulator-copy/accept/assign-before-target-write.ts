items.reduce((acc) => {
  const holder = { target: {}, other: {} };
  holder.other = shared;
  const copy = Object.assign(holder.target, acc);
  holder.target = shared;
  return copy;
}, {});
