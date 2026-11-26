const shared = {};
[1].reduce(acc => {
  const holder = { target: {} };
  return Object.assign(holder[(holder.target = shared, 'target')], acc);
}, {});
