-- Backend regressions: scalar values and colliding public/module names.
fun scalar_sext (x: u1, o: out u8)
  o = @unsigned(@sext(x, 8))
fun scalar_trunc (x: u1, o: out u1)
  o = @trunc(x, 1)
fun scalar_slice (x: u1, o: out u1)
  o = x[0]
fun scalar_expr (x: u1, o: out u8)
  o = @unsigned(@sext(!x, 8))
fun scalar_dynamic (x: u1, i: u2, o: out u1)
  o = x[i]
fun scalar_signed (x: i1, y: i1, o: out u1, sx: out u8)
  o = x[0] < y[0]
  sx = @unsigned(@sext(x, 8))
fun name_collision (cell: u8, cell_: u8, cell__1: u8, o: out u8)
  o = cell + cell_ + cell__1
process cell (src: buffer in u8, dst: buffer out u8)
  loop
    let x = @rcv(src)
    @send(dst, x + 8'd1)
process cell_ (src: buffer in u8, dst: buffer out u8)
  loop
    let x = @rcv(src)
    @send(dst, x + 8'd2)
graph collision_graph (src: buffer in u8, dst: buffer out u8)
  let middle: buffer u8
  cell(src, middle)
  cell_(middle, dst)
