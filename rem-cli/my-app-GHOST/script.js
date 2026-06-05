// ── Main ────────────────────────
document.addEventListener('DOMContentLoaded', () => {
    console.log('App ready');
});

// ── Navigation ───────────────────
document.querySelectorAll('nav a').forEach(link => {
    link.addEventListener('click', (e) => {
        e.preventDefault();
        console.log(`Navigate to: ${link.textContent}`);
    });
});
