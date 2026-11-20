type Id<T> = T;
type A = Id<Record<string, Id<unknown>>>;
