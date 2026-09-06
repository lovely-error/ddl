fun scalar_sext (x: u1, o: out u8)
  o = @unsigned(@sext(x, 8))

fun scalar_trunc (x: u1, o: out u1)
  o = @trunc(x, 1)

fun scalar_slice (x: u1, o: out u1)
  o = x[0]

fun name_collision (cell: u8, cell_: u8, o: out u8)
  o = cell + cell_
