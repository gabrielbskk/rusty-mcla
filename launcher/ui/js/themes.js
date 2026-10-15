// Theme ids and how a theme is applied. The colours themselves are CSS custom properties in css/themes.css,
// switched by one attribute on the root: <html data-theme="...">. Names and descriptions are in locales (theme.<id>.*).
(function () {
  const THEMES = [
    { id: 'steel', mode: 'dark' },
    { id: 'graphite', mode: 'dark' },
    { id: 'crimson', mode: 'dark' },
    { id: 'sodium', mode: 'dark' },
    { id: 'teal', mode: 'dark' },
    { id: 'dracula', mode: 'dark' },
    { id: 'alucard', mode: 'light' },
    { id: 'system', mode: 'auto' },
  ];
  const DEFAULT_THEME = 'steel';
  const darkQuery = window.matchMedia ? window.matchMedia('(prefers-color-scheme: light)') : null;
  let current = DEFAULT_THEME;

  function isLight(id) {
    return id === 'alucard' || (id === 'system' && !!darkQuery && darkQuery.matches);
  }

  function apply(id) {
    if (!THEMES.some((t) => t.id === id)) id = DEFAULT_THEME;
    current = id;
    const root = document.documentElement;
    root.setAttribute('data-theme', id);
    root.setAttribute('data-scheme', isLight(id) ? 'light' : 'dark'); // scrollbars and form controls
    return id;
  }

  if (darkQuery && darkQuery.addEventListener) {
    darkQuery.addEventListener('change', () => apply(current));
  }

  window.Themes = { list: THEMES, defaultId: DEFAULT_THEME, apply, current: () => current };
})();
