type Command = () => void; const startCommand = () => {}; const source = { start: startCommand }; const commands: Record<string, Command> = source;
