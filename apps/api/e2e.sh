#!/usr/bin/env bash
# E2E contra el servidor real ya corriendo (cargo run). Uso: bash e2e.sh
BASE="${BASE_URL:-http://localhost:8080}"
USER_NAME="${API_USER:-admin}"
PASS="${API_PASSWORD:-mundial2026}"
FAILS=0

ok()   { echo "PASS  $1"; }
fail() { echo "FAIL  $1  ($2)"; FAILS=$((FAILS + 1)); }
check() { # check "descripcion" "esperado" "obtenido"
  if [ "$2" = "$3" ]; then ok "$1"; else fail "$1" "esperado=$2 obtenido=$3"; fi
}
total_of() { echo "$1" | grep -o '"total":[0-9]*' | head -1 | cut -d: -f2; }

# 1. login
LOGIN=$(curl -s -w '\n%{http_code}' -X POST "$BASE/api/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER_NAME\",\"password\":\"$PASS\"}")
check "login 200" "200" "$(echo "$LOGIN" | tail -1)"
TOKEN=$(echo "$LOGIN" | head -1 | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
[ -n "$TOKEN" ] && ok "login devuelve token" || { fail "login devuelve token" "vacio"; exit 1; }
AUTH="Authorization: Bearer $TOKEN"

BAD=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/auth/login" \
  -H 'Content-Type: application/json' -d '{"username":"admin","password":"mala"}')
check "login con clave incorrecta 401" "401" "$BAD"

# 2. exactamente 20 polizas sembradas (POL-0001..POL-0020)
ALL=$(curl -s -H "$AUTH" "$BASE/api/policies")
SEEDED=$(echo "$ALL" | grep -oE '"id":"POL-00(0[1-9]|1[0-9]|20)"' | wc -l | tr -d ' ')
check "20 polizas sembradas (POL-0001..POL-0020)" "20" "$SEEDED"
TOTAL_BEFORE=$(total_of "$ALL")

# 3. filtro por status
ACT=$(curl -s -H "$AUTH" "$BASE/api/policies?status=ACTIVA")
CAN=$(curl -s -H "$AUTH" "$BASE/api/policies?status=CANCELADA")
T_ACT=$(total_of "$ACT"); T_CAN=$(total_of "$CAN")
check "ACTIVA + CANCELADA = total" "$TOTAL_BEFORE" "$((T_ACT + T_CAN))"
[ "$T_ACT" -gt 0 ] && [ "$T_CAN" -gt 0 ] && ok "hay polizas de ambos estados" || fail "hay polizas de ambos estados" "ACTIVA=$T_ACT CANCELADA=$T_CAN"
echo "$ACT" | grep -q '"status":"CANCELADA"' && fail "filtro ACTIVA no mezcla" "contiene CANCELADA" || ok "filtro ACTIVA no mezcla estados"
echo "$CAN" | grep -q '"status":"ACTIVA"' && fail "filtro CANCELADA no mezcla" "contiene ACTIVA" || ok "filtro CANCELADA no mezcla estados"

# 4. crear poliza
CREATED=$(curl -s -w '\n%{http_code}' -X POST "$BASE/api/policies" -H "$AUTH" \
  -H 'Content-Type: application/json' \
  -d '{"clientName":"Cliente E2E","branch":"HOGAR","monthlyPremiumCop":123000,"startDate":"2026-01-01","endDate":"2027-01-01"}')
check "crear poliza 201" "201" "$(echo "$CREATED" | tail -1)"
NEW_ID=$(echo "$CREATED" | head -1 | sed -n 's/.*"id":"\(POL-[0-9]*\)".*/\1/p')
[ -n "$NEW_ID" ] && ok "poliza creada con id $NEW_ID" || fail "poliza creada con id" "vacio"
echo "$CREATED" | head -1 | grep -q '"status":"ACTIVA"' && ok "status por defecto ACTIVA" || fail "status por defecto ACTIVA" "$CREATED"

AFTER=$(curl -s -H "$AUTH" "$BASE/api/policies")
check "total incrementa en 1" "$((TOTAL_BEFORE + 1))" "$(total_of "$AFTER")"
echo "$AFTER" | grep -q "\"id\":\"$NEW_ID\"" && ok "poliza aparece en la lista" || fail "poliza aparece en la lista" "$NEW_ID"
check "detalle de la poliza nueva 200" "200" "$(curl -s -o /dev/null -w '%{http_code}' -H "$AUTH" "$BASE/api/policies/$NEW_ID")"
check "poliza inexistente 404" "404" "$(curl -s -o /dev/null -w '%{http_code}' -H "$AUTH" "$BASE/api/policies/POL-9999")"

# 5. validacion: prima 0 -> 400
INV=$(curl -s -w '\n%{http_code}' -X POST "$BASE/api/policies" -H "$AUTH" \
  -H 'Content-Type: application/json' \
  -d '{"clientName":"Cliente E2E","branch":"VIDA","monthlyPremiumCop":0,"startDate":"2026-01-01","endDate":"2027-01-01"}')
check "prima 0 -> 400" "400" "$(echo "$INV" | tail -1)"
echo "$INV" | head -1 | grep -q '"fields":{[^}]*"monthlyPremiumCop"' && ok "400 incluye fields.monthlyPremiumCop" || fail "400 incluye fields.monthlyPremiumCop" "$INV"

# 6. sin token -> 401
check "listar sin token 401" "401" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/policies")"
check "crear sin token 401" "401" "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/policies" -H 'Content-Type: application/json' -d '{}')"

echo
if [ "$FAILS" -eq 0 ]; then echo "E2E OK"; else echo "E2E FALLO: $FAILS verificaciones"; exit 1; fi
