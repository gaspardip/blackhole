type Command = () => void; const startCommand = () => {}; type Index<T = Command> = Record<string, T>; const commands: Index = { start: startCommand };
