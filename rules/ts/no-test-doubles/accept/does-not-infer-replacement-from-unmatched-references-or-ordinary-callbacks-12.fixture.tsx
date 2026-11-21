const original = document.onclick; document.onclick = () => {}; document.onclick = original;
