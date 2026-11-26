declare const items: number[];
items.reduce((acc) => {
  const holder = { target: {} };
  holder.target = shared;
  return Object.assign(holder.target, acc);
}, {});
items.reduce((acc) => {
  const holder = { target: {} };
  const alias = holder;
  ({ target: alias["target"] } = state);
  return Object.assign(holder.target, acc);
}, {});
