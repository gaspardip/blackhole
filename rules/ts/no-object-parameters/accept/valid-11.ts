type Key = object; type Mapped<Input> = { [Key in keyof Input]: (value: Key) => void };
