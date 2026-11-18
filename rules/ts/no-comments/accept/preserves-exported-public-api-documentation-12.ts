interface Result { /** Number of items. */ count: number; }
export function count(): Result { return { count: 1 }; }
