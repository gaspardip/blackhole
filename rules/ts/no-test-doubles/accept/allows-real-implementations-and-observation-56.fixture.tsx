let { defineProperty: define } = Object; define = custom; define(navigator, 'clipboard', { value: clipboard });
