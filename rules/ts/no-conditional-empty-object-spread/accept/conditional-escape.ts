function build(flag: boolean) {
  const row = {};
  Object.assign(flag ? row : {}, { id: 1 });
  return { ...(flag ? row : { id: 2 }) };
}
function escaped(c, x) {
  const empty = {};
  const arm = c ? empty : x;
  mutate(arm);
  consume({ ...arm });
  consume({ ...(c ? empty : x) });
}
