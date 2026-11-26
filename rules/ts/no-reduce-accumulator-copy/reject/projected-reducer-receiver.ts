const page = { rows: [1] };
const holder = { rows: page.rows };
holder.rows.reduce.call(page.rows, acc => [...acc], []);
