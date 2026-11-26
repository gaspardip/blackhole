declare const page: { rows: number[] };
function copy(acc: number[], row: number) {
  return [...acc, row];
}
page.rows.reduce.call(page.rows, copy, []);
const rows = page.rows;
page.rows.reduce.call(rows, (acc, row) => [...acc, row], []);
