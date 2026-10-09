const BRANCHES = ["AUTOMOVILES", "HOGAR", "VIDA"];
const STATUSES = ["ACTIVA", "CANCELADA"];
const COLS = "id, client_name AS clientName, branch, monthly_premium_cop AS monthlyPremiumCop, start_date AS startDate, end_date AS endDate, status, created_at AS createdAt";

const json = (body, status = 200) => Response.json(body, { status });
const err = (status, error, extra = {}) => json({ error, ...extra }, status);

async function tokenFor(env) {
  const data = new TextEncoder().encode(`${env.API_USER}:${env.API_PASSWORD ?? "mundial2026"}:policy-lab`);
  const hash = await crypto.subtle.digest("SHA-256", data);
  return [...new Uint8Array(hash)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

const validDate = (s) => typeof s === "string" && /^\d{4}-\d{2}-\d{2}$/.test(s) && !isNaN(Date.parse(s));

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    const path = url.pathname;

    if (path === "/api/auth/login" && req.method === "POST") {
      const b = await req.json().catch(() => ({}));
      if (b.username !== env.API_USER || b.password !== (env.API_PASSWORD ?? "mundial2026"))
        return err(401, "Credenciales invalidas");
      return json({ token: await tokenFor(env) });
    }

    if (!path.startsWith("/api/policies")) return err(404, "No encontrado");
    const auth = req.headers.get("authorization") ?? "";
    if (auth.replace(/^Bearer\s+/, "") !== (await tokenFor(env))) return err(401, "Token invalido o ausente");

    if (path === "/api/policies" && req.method === "GET") {
      const status = url.searchParams.get("status");
      const order = " ORDER BY created_at DESC, id DESC";
      const { results } = status
        ? await env.DB.prepare(`SELECT ${COLS} FROM policies WHERE status = ?1${order}`).bind(status).all()
        : await env.DB.prepare(`SELECT ${COLS} FROM policies${order}`).all();
      return json({ items: results, total: results.length });
    }

    if (path === "/api/policies" && req.method === "POST") {
      const b = await req.json().catch(() => ({}));
      const fields = {};
      const clientName = String(b.clientName ?? "").trim();
      if (!clientName) fields.clientName = "El nombre del cliente es obligatorio";
      if (!BRANCHES.includes(b.branch)) fields.branch = "Debe ser AUTOMOVILES, HOGAR o VIDA";
      if (!Number.isInteger(b.monthlyPremiumCop) || b.monthlyPremiumCop <= 0) fields.monthlyPremiumCop = "Debe ser un entero mayor a cero";
      if (!validDate(b.startDate)) fields.startDate = "Fecha invalida, use YYYY-MM-DD";
      if (!validDate(b.endDate)) fields.endDate = "Fecha invalida, use YYYY-MM-DD";
      else if (validDate(b.startDate) && b.endDate <= b.startDate) fields.endDate = "La fecha de fin debe ser posterior a la de inicio";
      const status = b.status ?? "ACTIVA";
      if (!STATUSES.includes(status)) fields.status = "Debe ser ACTIVA o CANCELADA";
      if (Object.keys(fields).length) return err(400, "Datos invalidos", { fields });

      const last = await env.DB.prepare("SELECT id FROM policies ORDER BY id DESC LIMIT 1").first("id");
      const id = `POL-${String((parseInt(String(last ?? "POL-0000").slice(4)) || 0) + 1).padStart(4, "0")}`;
      const createdAt = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
      await env.DB.prepare("INSERT INTO policies VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
        .bind(id, clientName, b.branch, b.monthlyPremiumCop, b.startDate, b.endDate, status, createdAt).run();
      return json({ id, clientName, branch: b.branch, monthlyPremiumCop: b.monthlyPremiumCop, startDate: b.startDate, endDate: b.endDate, status, createdAt }, 201);
    }

    const m = path.match(/^\/api\/policies\/([^/]+)$/);
    if (m && req.method === "GET") {
      const p = await env.DB.prepare(`SELECT ${COLS} FROM policies WHERE id = ?1`).bind(decodeURIComponent(m[1])).first();
      return p ? json(p) : err(404, "Poliza no encontrada");
    }
    return err(405, "Metodo no permitido");
  },
};
