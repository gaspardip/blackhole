type Item = object; type Fallback<Input> = Input extends infer Item ? string : (value: Item) => void;
