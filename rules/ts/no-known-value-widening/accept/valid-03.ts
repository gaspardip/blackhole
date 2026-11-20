type Command = () => void; const startCommand = () => {}; class Registry { commands: Record<string, Command> = {}; }
