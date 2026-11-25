const empty = {};
void empty;
consume({ ...(ready ? empty : { id: 2 }) });
