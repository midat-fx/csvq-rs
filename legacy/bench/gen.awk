# gen.awk — generate a deterministic CSV of --rows data rows for benchmarking.
# Usage: awk -v rows=1000000 -v seed=42 -f bench/gen.awk > big.csv
BEGIN {
  if (rows == "") rows = 100000
  if (seed == "") seed = 42
  state = seed
  print "id,name,city,amount,active"
  ncities = split("Almaty,Astana,Shymkent,Aktobe,Karaganda,Taraz", C, ",")
  for (i = 1; i <= rows; i++) {
    state = (1664525 * state + 1013904223) % 4294967296
    city = C[(int(state / 65536) % ncities) + 1]
    amt = sprintf("%.2f", (state % 100000) / 100.0)
    act = (state % 2 == 0) ? "true" : "false"
    printf "%d,name_%d,%s,%s,%s\n", i, i, city, amt, act
  }
}
