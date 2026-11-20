type Slot<T> = T;
type Shape = Slot<{[k: string]: Slot<unknown>}>;
