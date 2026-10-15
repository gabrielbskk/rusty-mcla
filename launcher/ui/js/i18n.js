// Language support. All visible text lives in locales/<code>.json (flat key -> string); the code holds no sentences.
// English is always loaded first and every other language is laid over it, so a missing key falls back to English
// one key at a time. Adding a language = a locales/<code>.json plus one line in locales/languages.json.
(function () {
  let en = {};
  let over = {};
  let lang = 'en';
  const missing = new Set();

  async function getJson(url) {
    const r = await fetch(url, { cache: 'no-store' });
    if (!r.ok) throw new Error(url + ' ' + r.status);
    return r.json();
  }

  async function languages() {
    try {
      return await getJson('locales/languages.json');
    } catch (e) {
      return [{ code: 'en', name: 'English' }];
    }
  }

  // "auto" = the system language when it is shipped, else English.
  function resolve(setting, shipped) {
    const codes = shipped.map((l) => l.code);
    if (setting && setting !== 'auto' && codes.includes(setting)) return setting;
    const sys = ((navigator.languages && navigator.languages[0]) || navigator.language || 'en').toLowerCase();
    const base = sys.split('-')[0];
    return codes.includes(sys) ? sys : codes.includes(base) ? base : 'en';
  }

  async function load(code) {
    if (!Object.keys(en).length) en = await getJson('locales/en.json');
    over = {};
    lang = 'en';
    if (code && code !== 'en') {
      try {
        over = await getJson('locales/' + code + '.json');
        lang = code;
      } catch (e) {
        over = {};
      }
    }
    document.documentElement.setAttribute('lang', lang);
  }

  function t(key, args) {
    let s = Object.prototype.hasOwnProperty.call(over, key) ? over[key] : en[key];
    if (s === undefined) {
      missing.add(key);
      return key;
    }
    if (args) s = s.replace(/\{(\d+)\}/g, (m, i) => (args[i] !== undefined ? args[i] : m));
    return s;
  }

  function apply(root) {
    root = root || document;
    root.querySelectorAll('[data-i18n]').forEach((el) => {
      el.textContent = t(el.getAttribute('data-i18n'));
    });
    root.querySelectorAll('[data-i18n-attr]').forEach((el) => {
      el.getAttribute('data-i18n-attr').split(';').forEach((pair) => {
        const i = pair.indexOf(':');
        if (i > 0) el.setAttribute(pair.slice(0, i).trim(), t(pair.slice(i + 1).trim()));
      });
    });
  }

  // Test hook: lay a partial language over English without fetching.
  function useStrings(base, overlay) {
    en = base;
    over = overlay || {};
  }

  window.I18N = { languages, resolve, load, t, apply, useStrings, missing: () => Array.from(missing), current: () => lang, table: () => en };
})();
