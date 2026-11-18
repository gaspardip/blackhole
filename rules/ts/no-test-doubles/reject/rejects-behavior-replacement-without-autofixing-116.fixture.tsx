const key = "clipboard"; const replacement = { [key]: { value: clipboard } }; Object.defineProperties(navigator, { ...replacement });
