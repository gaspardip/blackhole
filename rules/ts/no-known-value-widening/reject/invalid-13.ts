type Command = () => void; const startCommand = () => {}; function create(): Record<string, Command> { return { start: startCommand }; }
