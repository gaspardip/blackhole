type Command = () => void; const startCommand = () => {}; type Index<T> = Record<string, T>; const commands: Index<Command> = { start: startCommand };
