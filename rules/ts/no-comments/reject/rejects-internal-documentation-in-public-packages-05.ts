interface Local { /** Internal field. */ value: number; }
export function count() { const local: Local = { value: 1 }; return local.value; }
