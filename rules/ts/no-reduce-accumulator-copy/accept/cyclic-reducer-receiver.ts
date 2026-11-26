const a = b.loop;
const b = a.loop;
a.rows.reduce.call(b.rows, acc => [...acc], []);
