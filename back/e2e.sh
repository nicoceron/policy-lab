#!/usr/bin/env bash
# Prueba e2e con curl contra el servidor real (cargo run) en localhost:8080.
set -u

API="${API:-http://localhost:8080}"
USER_NAME="${API_USER:-admin}"
PASSWORD="${API_PASSWORD:-mundial2026}"
FAILED=0

check() {
  if [ "$2" = "$3" ]; then
    echo "OK   $1 ($2)"
  else
    echo "FALLA $1: esperado '$3', recibido '$2'"
    FAILED=1
  fi
}

total_of() { grep -o '"total":[0-9]*' | head -n 1 | cut -d: -f2; }

echo "== 1. Login =="
LOGIN=$(curl -s -X POST "$API/api/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER_NAME\",\"password\":\"$PASSWORD\"}")
TOKEN=$(echo "$LOGIN" | grep -o '"token":"[^"]*"' | cut -d'"' -f4)
if [ -n "$TOKEN" ]; then echo "OK   token recibido"; else echo "FALLA login: $LOGIN"; exit 1; fi

BAD_LOGIN=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$API/api/auth/login" \
  -H 'Content-Type: application/json' -d '{"username":"admin","password":"nope"}')
check "credenciales invalidas -> 401" "$BAD_LOGIN" "401"

echo "== 2. Sin token =="
NO_TOKEN=$(curl -s -o /dev/null -w '%{http_code}' "$API/api/policies")
check "GET /api/policies sin token -> 401" "$NO_TOKEN" "401"

echo "== 3. Las 20 polizas sembradas =="
LIST=$(curl -s "$API/api/policies" -H "Authorization: Bearer $TOKEN")
TOTAL_BEFORE=$(echo "$LIST" | total_of)
SEEDED=0
for i in $(seq -f '%04g' 1 20); do
  echo "$LIST" | grep -q "\"id\":\"POL-$i\"" && SEEDED=$((SEEDED + 1))
done
check "polizas sembradas presentes" "$SEEDED" "20"

echo "== 4. Filtro por estado =="
ACT=$(curl -s "$API/api/policies?status=ACTIVA" -H "Authorization: Bearer $TOKEN")
CAN=$(curl -s "$API/api/policies?status=CANCELADA" -H "Authorization: Bearer $TOKEN")
TOTAL_ACT=$(echo "$ACT" | total_of)
TOTAL_CAN=$(echo "$CAN" | total_of)
check "ACTIVA + CANCELADA = total" "$((TOTAL_ACT + TOTAL_CAN))" "$TOTAL_BEFORE"
check "filtro ACTIVA sin canceladas" "$(echo "$ACT" | grep -c '"status":"CANCELADA"')" "0"

echo "== 5. Crear poliza =="
CREATED=$(curl -s -w '\n%{http_code}' -X POST "$API/api/policies" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"clientName":"Prueba E2E","branch":"HOGAR","monthlyPremiumCop":150000,"startDate":"2026-01-01","endDate":"2027-01-01"}')
CREATE_CODE=$(echo "$CREATED" | tail -n 1)
NEW_ID=$(echo "$CREATED" | head -n 1 | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
check "POST /api/policies -> 201" "$CREATE_CODE" "201"

LIST2=$(curl -s "$API/api/policies" -H "Authorization: Bearer $TOKEN")
check "la nueva poliza aparece en la lista" \
  "$(echo "$LIST2" | grep -c "\"id\":\"$NEW_ID\"")" "1"
check "total incrementa en 1" "$(echo "$LIST2" | total_of)" "$((TOTAL_BEFORE + 1))"

DETAIL=$(curl -s -o /dev/null -w '%{http_code}' "$API/api/policies/$NEW_ID" \
  -H "Authorization: Bearer $TOKEN")
check "GET /api/policies/:id -> 200" "$DETAIL" "200"

echo "== 6. Prima invalida =="
BAD=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$API/api/policies" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"clientName":"Prima Cero","branch":"VIDA","monthlyPremiumCop":0,"startDate":"2026-01-01","endDate":"2027-01-01"}')
check "prima en 0 -> 400" "$BAD" "400"

echo
if [ "$FAILED" -eq 0 ]; then
  echo "Todas las pruebas e2e pasaron."
else
  echo "Hubo pruebas e2e fallidas."
fi
exit "$FAILED"
