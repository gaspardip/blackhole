type Command = () => void; const startCommand = () => {}; class Registry { accessor commands: Record<string, Command> = {}; }
