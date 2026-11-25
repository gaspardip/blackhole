consume({ ...(ready ? { id: 1 } : fallback ? {} : { id: 2 }) });
const inner = fallback ? {} : {};
consume({ ...(ready ? { id: 1 } : inner) });
