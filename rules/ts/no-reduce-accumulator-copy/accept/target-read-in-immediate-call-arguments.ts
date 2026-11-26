declare const items: number[];
items.reduce(acc => {
  const holder = { target: {} };
  return (target => {
    holder.target = shared;
    return target;
  })(Object.assign(holder.target, acc));
}, {});
