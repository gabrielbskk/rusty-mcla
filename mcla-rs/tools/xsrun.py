"""Run a program translated by tools/xs2wgsl.py on numpy arrays (a whole image at once): the
WGSL the translator emits is simple enough to be executed as Python after a few rewrites.
    from xsrun import run;  out = run(wgsl_source, 'xs_perlin_ps', rin, consts, samplers)
rin: dict register -> (H, W, 4) array; consts: dict index -> 4 floats; samplers: dict name -> f(uv)
(uv an (H, W, 2) array) returning (H, W, 4)."""
import re, numpy as np


class V:
    """a vec of 2..4 components, each an array (or a float)"""
    def __init__(self, *c):
        self.c = list(c)

    def __getattr__(self, name):
        if name.startswith('_') or name == 'c':
            raise AttributeError(name)
        idx = ['xyzw'.index(ch) for ch in name]
        return self.c[idx[0]] if len(idx) == 1 else V(*[self.c[i] for i in idx])

    def __setattr__(self, name, value):
        if name == 'c':
            object.__setattr__(self, name, value)
        elif len(name) == 1:
            self.c['xyzw'.index(name)] = value
        else:
            for ch, v in zip(name, value.c):
                self.c['xyzw'.index(ch)] = v

    def _b(self, o, f):
        if isinstance(o, V):
            return V(*[f(a, b) for a, b in zip(self.c, o.c)])
        return V(*[f(a, o) for a in self.c])

    def __add__(self, o): return self._b(o, lambda a, b: a + b)
    def __radd__(self, o): return self._b(o, lambda a, b: b + a)
    def __sub__(self, o): return self._b(o, lambda a, b: a - b)
    def __rsub__(self, o): return self._b(o, lambda a, b: b - a)
    def __mul__(self, o): return self._b(o, lambda a, b: a * b)
    def __rmul__(self, o): return self._b(o, lambda a, b: b * a)
    def __truediv__(self, o): return self._b(o, lambda a, b: a / b)
    def __neg__(self): return V(*[-a for a in self.c])
    def __gt__(self, o): return self._b(o, lambda a, b: a > b)
    def __ge__(self, o): return self._b(o, lambda a, b: a >= b)
    def __lt__(self, o): return self._b(o, lambda a, b: a < b)
    def __le__(self, o): return self._b(o, lambda a, b: a <= b)
    def __eq__(self, o): return self._b(o, lambda a, b: a == b)
    def __ne__(self, o): return self._b(o, lambda a, b: a != b)
    __hash__ = None


def vec(*a):
    out = []
    for x in a:
        out += x.c if isinstance(x, V) else [x]
    if len(out) == 1:
        out = out * 4
    return V(*out)


def _un(f):
    return lambda a: V(*[f(c) for c in a.c]) if isinstance(a, V) else f(a)


def _bi(f):
    def g(a, b):
        if isinstance(a, V) or isinstance(b, V):
            a = a if isinstance(a, V) else V(*[a] * len(b.c))
            b = b if isinstance(b, V) else V(*[b] * len(a.c))
            return V(*[f(x, y) for x, y in zip(a.c, b.c)])
        return f(a, b)
    return g


def select(f, t, c):
    if isinstance(c, V):
        f = f if isinstance(f, V) else V(*[f] * len(c.c))
        t = t if isinstance(t, V) else V(*[t] * len(c.c))
        return V(*[np.where(k, a, b) for k, a, b in zip(c.c, t.c, f.c)])
    if isinstance(f, V) or isinstance(t, V):
        n = len((f if isinstance(f, V) else t).c)
        f = f if isinstance(f, V) else V(*[f] * n)
        t = t if isinstance(t, V) else V(*[t] * n)
        return V(*[np.where(c, a, b) for a, b in zip(t.c, f.c)])
    return np.where(c, t, f)


def _clamp(x, lo, hi):
    return _bi(np.minimum)(_bi(np.maximum)(x, lo), hi)


ENV = {
    'vec4': vec, 'vec3': vec, 'vec2': vec, 'select': select, 'np': np,
    'max': _bi(np.maximum), 'min': _bi(np.minimum), 'abs': _un(np.abs), 'floor': _un(np.floor), 'trunc': _un(np.trunc),
    'fract': _un(lambda x: x - np.floor(x)),
    'sqrt': _un(lambda x: np.sqrt(np.maximum(x, 0))), 'inverseSqrt': _un(lambda x: 1 / np.sqrt(np.maximum(x, 1e-30))),
    'exp2': _un(np.exp2), 'log2': _un(lambda x: np.log2(np.maximum(x, 1e-30))), 'sin': _un(np.sin), 'cos': _un(np.cos),
    'dot': lambda a, b: sum(x * y for x, y in zip(a.c, b.c)),
    'clamp': _clamp, 'f32': float,
}


def python_lines(src, fn):
    i = src.index('fn %s(' % fn)
    body = src[i:src.index('\n}\n', i)].split('\n')[1:]
    py = []
    cond = None
    for ln in body:
        s = ln.strip()
        if not s or s.startswith('//') or s.startswith('var ') or s.startswith('for (') or s == '{' or s.startswith('{ //'):
            continue
        if s.startswith('return'):
            continue
        if s == '}':
            cond = None
            continue
        if s.startswith('if ('):
            # a predicated write: "if (cond) {" ... "}" - each assignment inside becomes a select
            c = s[3:s.rindex('{')].strip()
            cond = c.replace('&&', '&').replace('!', '~')
            continue
        s = s.split('//')[0].strip().rstrip(';')
        s = s.replace('vec4<f32>', 'vec4').replace('vec3<f32>', 'vec3').replace('vec2<f32>', 'vec2')
        s = re.sub(r'^(let|var) (\w+)(: [\w<>]+)? = ', r'\2 = ', s)
        s = re.sub(r'xs_c\[(\d+)\]', r'C[\1]', s)
        s = s.replace('!p', '~p')
        if cond is not None and ' = ' in s:
            lhs, rhs = s.split(' = ', 1)
            s = '%s = select(%s, %s, %s)' % (lhs, lhs, rhs, cond)
        py.append(s)
    return py


def run(src, fn, rin, consts, samplers, trace=None):
    py = python_lines(src, fn)
    r = [V(0.0, 0.0, 0.0, 0.0) for _ in range(64)]
    shape = None
    for k, a in rin.items():
        r[k] = V(*[a[..., j] for j in range(4)])
        shape = a.shape[:-1]
    env = dict(ENV)
    env.update({'r': r, 'o': [V(0.0, 0.0, 0.0, 0.0) for _ in range(16)], 'ps': 0.0, 'p': np.array(False), 'pb': np.array(False),
                'C': {k: V(*v) for k, v in consts.items()}})
    for name, f in samplers.items():
        def make(f):
            def g(uv, *more):
                a = f(np.stack([np.broadcast_to(c, shape) for c in uv.c[:2]], axis=-1))
                return V(*[a[..., j] for j in range(4)])
            return g
        env[name] = make(f)
    for n, s in enumerate(py):
        try:
            exec(s, env)
        except Exception as e:
            raise SystemExit('xsrun: line %d: %s\n  %s' % (n, s, e))
        if trace:
            trace(n, s, env)
    o = env['o'][0]
    return np.stack([np.broadcast_to(np.asarray(c, dtype=np.float64), shape) for c in o.c], axis=-1)


def run_vs(src, fn, vin, consts):
    """A vertex program: vin a list of (H, W, 4) arrays; returns the list of its 16 outputs."""
    py = python_lines(src, fn)
    shape = vin[0].shape[:-1]
    env = dict(ENV)
    env.update({'r': [V(0.0, 0.0, 0.0, 0.0) for _ in range(64)], 'o': [V(0.0, 0.0, 0.0, 0.0) for _ in range(16)], 'opos': V(0.0, 0.0, 0.0, 0.0),
                'ps': 0.0, 'p': np.array(False), 'pb': np.array(False), 'C': {k: V(*v) for k, v in consts.items()},
                'vin': [V(*[a[..., j] for j in range(4)]) for a in vin] + [V(0.0, 0.0, 0.0, 0.0)] * 16})
    for n, s in enumerate(py):
        try:
            exec(s, env)
        except Exception as e:
            raise SystemExit('xsrun: line %d: %s: %s' % (n, s, e))
    return [np.stack([np.broadcast_to(np.asarray(c, dtype=np.float64), shape) for c in o.c], axis=-1) for o in env['o']]
