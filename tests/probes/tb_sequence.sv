`timescale 1ns/1ps
module tb_sequence;
  reg clk=0; always #5 clk=~clk;
  reg rst_n=0;
  reg [1:0] ws=0;
  reg [31:0] input_data=0;
  wire [1:0] sr,sw,ar,aw,mr,mw,vr,vw,lr,lw;
  wire [31:0] sd,ad,md,vd,ld;
  seq_scope s(clk,rst_n,ws,sr,input_data,sw,2'b00,sd);
  seq_address a(clk,rst_n,ws,ar,input_data,aw,2'b00,ad);
  seq_read_first m(clk,rst_n,ws,mr,8'd0,mw,2'b00,md);
  seq_assert v(clk,rst_n,ws,vr,input_data,vw,2'b00,vd);
  seq_literal l(clk,rst_n,ws,lr,input_data,lw,2'b00,ld);

  // ---- several pipes on one sequence --------------------------------------
  //
  // The head is a join and the tail a scatter, and the property neither the
  // Rust interpreter nor a golden file states well is the SIMULTANEITY: both
  // outputs are written on the same edge, from the same item, or neither is.
  reg [1:0] jaws=0, jbws=0;
  reg [31:0] ja_data=0, jb_data=0;
  wire [1:0] jar,jbr,jsw,jdw;
  wire [31:0] jsd,jdd;
  seq_join j(clk,rst_n,jaws,jar,ja_data,jbws,jbr,jb_data,
             jsw,2'b00,jsd,jdw,2'b00,jdd);

  // An optional input: `b` never offers anything, and the pipeline must run
  // anyway and report `ok` low by leaving `x` alone.
  reg [1:0] oaws=0;
  reg [31:0] oa_data=0;
  wire [1:0] oar,obr,oow;
  wire [31:0] ood;
  seq_optional o(clk,rst_n,oaws,oar,oa_data,2'b00,obr,32'd0,oow,2'b00,ood);

  // The same shape with the optional input actually delivering, and delivering
  // LESS than the blocking one: `a` carries two items and `b` one, so the pair
  // that fires first has its `ok` high and the second has it low. That the two
  // items get different answers is the whole of what `ok` means -- an `ok`
  // stuck high or low would pass a test where `b` was always full or always
  // empty, and this is the one it cannot pass.
  reg [1:0] paws=0, pbws=0;
  reg [31:0] pa_data=0, pb_data=0;
  wire [1:0] par,pbr,pow;
  wire [31:0] pod;
  seq_optional p(clk,rst_n,paws,par,pa_data,pbws,pbr,pb_data,pow,2'b00,pod);

  // ---- sends from different stages, joined downstream ---------------------
  //
  // `x` leaves in stage 0 and `y` two stages later, and the consumer below
  // takes from both together or from neither, stalling on an irregular
  // pattern. It cannot drain `x` until the matching `y` arrives, so `x` fills
  // while that item is still in the pipeline. One shift for the whole
  // pipeline deadlocks here after two items; per-stage shifts let the stages
  // below keep moving, so every item arrives, paired and in order.
  localparam EARLY_N = 24;
  reg [1:0] ews=0, erx=0, ery=0;
  reg [15:0] ee0=0, ee1=0;
  wire [1:0] er, exw, eyw;
  wire [31:0] exd, eyd;
  seq_early e(clk,rst_n,ews,er,{ee1,ee0},exw,erx,exd,eyw,ery,eyd);
  reg early_go=0;
  integer early_sent=0, early_got=0, early_cycle=0;
  wire ews_idx = ews[0]^ews[1];
  wire erx_idx = erx[0]^erx[1];
  wire ery_idx = ery[0]^ery[1];
  wire [15:0] ex_item = erx_idx ? exd[31:16] : exd[15:0];
  wire [15:0] ey_item = ery_idx ? eyd[31:16] : eyd[15:0];
  always @(negedge clk) if (early_go) begin
    early_cycle = early_cycle + 1;
    if (ews != ~er && early_sent < EARLY_N) begin
      if (ews_idx) ee1 <= early_sent + 1; else ee0 <= early_sent + 1;
      ews <= ews ^ (ews_idx ? 2'd2 : 2'd1);
      early_sent = early_sent + 1;
    end
    if ((early_cycle % 13) < 9 && exw != erx && eyw != ery) begin
      if (ex_item !== early_got + 1) $fatal(1,"early output out of order: %0d, wanted %0d",ex_item,early_got+1);
      if (ey_item !== early_got + 3) $fatal(1,"late output not paired: %0d, wanted %0d",ey_item,early_got+3);
      erx <= erx ^ (erx_idx ? 2'd2 : 2'd1);
      ery <= ery ^ (ery_idx ? 2'd2 : 2'd1);
      early_got = early_got + 1;
    end
  end

  // ---- nonblocking inputs below the head ----------------------------------
  //
  // `seq_side` samples `b` in stage 1 for the item passing through. `b` offers
  // sparsely and the consumer stalls on a pattern, so stage 1 is by turns
  // empty, moving and held while `b` has something. Every item must arrive in
  // order, the ones that carry a `b` item must carry them in order with none
  // repeated or skipped, and `b` must have been stepped once for each -- a
  // stage that took on a bubble or while held would break the last two.
  localparam SIDE_N = 40, SIDE_B = 12;
  reg [1:0] sws=0, sbws=0, sorx=0;
  reg [15:0] se0=0, se1=0, sb0=0, sb1=0;
  wire [1:0] sr_side, sbr, sow;
  wire [63:0] sod;
  seq_side sd_side(clk,rst_n,sws,sr_side,{se1,se0},sbws,sbr,{sb1,sb0},sow,sorx,sod);
  reg side_go=0;
  reg [1:0] sbr_last=0;
  integer side_sent=0, side_bsent=0, side_got=0, side_bgot=0, side_bsteps=0, side_cycle=0;
  wire sws_idx = sws[0]^sws[1];
  wire sbws_idx = sbws[0]^sbws[1];
  wire sorx_idx = sorx[0]^sorx[1];
  wire [31:0] so_item = sorx_idx ? sod[63:32] : sod[31:0];
  always @(negedge clk) if (side_go) begin
    side_cycle = side_cycle + 1;
    if (sbr !== sbr_last) begin side_bsteps = side_bsteps + 1; sbr_last = sbr; end
    if (sws != ~sr_side && side_sent < SIDE_N && (side_cycle % 3) != 0) begin
      if (sws_idx) se1 <= side_sent + 1; else se0 <= side_sent + 1;
      sws <= sws ^ (sws_idx ? 2'd2 : 2'd1);
      side_sent = side_sent + 1;
    end
    if (sbws != ~sbr && side_bsent < SIDE_B && (side_cycle % 3) == 1) begin
      if (sbws_idx) sb1 <= 1001 + side_bsent; else sb0 <= 1001 + side_bsent;
      sbws <= sbws ^ (sbws_idx ? 2'd2 : 2'd1);
      side_bsent = side_bsent + 1;
    end
    if ((side_cycle % 11) < 6 && sow != sorx) begin
      if (so_item[15:0] !== side_got + 1) $fatal(1,"sideband item out of order: %0d, wanted %0d",so_item[15:0],side_got+1);
      if (so_item[31:16] != 0) begin
        if (so_item[31:16] !== 1001 + side_bgot) $fatal(1,"sideband sample out of order: %0d, wanted %0d",so_item[31:16],1001+side_bgot);
        side_bgot = side_bgot + 1;
      end
      sorx <= sorx ^ (sorx_idx ? 2'd2 : 2'd1);
      side_got = side_got + 1;
    end
  end

  // `seq_peek_drop` looks at `b` in stage 1 and drops its head only when it
  // equals the item passing. `b` holds 3, 7, 7, 20 and the items climb, so 3
  // and the first 7 are dropped and the second 7 blocks the rest for good.
  localparam PD_N = 24;
  reg [1:0] pdws=0, pdbws=0, pdrx=0;
  reg [15:0] pde0=0, pde1=0, pdb0=0, pdb1=0;
  wire [1:0] pdr, pdbr, pdow;
  wire [63:0] pdod;
  seq_peek_drop pd(clk,rst_n,pdws,pdr,{pde1,pde0},pdbws,pdbr,{pdb1,pdb0},pdow,pdrx,pdod);
  reg pd_go=0;
  reg [1:0] pdbr_last=0;
  integer pd_sent=0, pd_bsent=0, pd_got=0, pd_hits=0, pd_bsteps=0, pd_cycle=0;
  reg [15:0] pd_b [0:3];
  initial begin pd_b[0]=3; pd_b[1]=7; pd_b[2]=7; pd_b[3]=20; end
  wire pdws_idx = pdws[0]^pdws[1];
  wire pdbws_idx = pdbws[0]^pdbws[1];
  wire pdrx_idx = pdrx[0]^pdrx[1];
  wire [31:0] pd_item = pdrx_idx ? pdod[63:32] : pdod[31:0];
  always @(negedge clk) if (pd_go) begin
    pd_cycle = pd_cycle + 1;
    if (pdbr !== pdbr_last) begin pd_bsteps = pd_bsteps + 1; pdbr_last = pdbr; end
    if (pdws != ~pdr && pd_sent < PD_N && (pd_cycle % 2) == 0) begin
      if (pdws_idx) pde1 <= pd_sent + 1; else pde0 <= pd_sent + 1;
      pdws <= pdws ^ (pdws_idx ? 2'd2 : 2'd1);
      pd_sent = pd_sent + 1;
    end
    if (pdbws != ~pdbr && pd_bsent < 4) begin
      if (pdbws_idx) pdb1 <= pd_b[pd_bsent]; else pdb0 <= pd_b[pd_bsent];
      pdbws <= pdbws ^ (pdbws_idx ? 2'd2 : 2'd1);
      pd_bsent = pd_bsent + 1;
    end
    if ((pd_cycle % 5) < 3 && pdow != pdrx) begin
      if (pd_item[15:0] !== pd_got + 1) $fatal(1,"peek/drop item out of order: %0d, wanted %0d",pd_item[15:0],pd_got+1);
      if ((pd_item[31:16] == 1) !== (pd_item[15:0] == 3 || pd_item[15:0] == 7)) $fatal(1,"peek/drop hit on the wrong item: %h",pd_item);
      if (pd_item[31:16] == 1) pd_hits = pd_hits + 1;
      pdrx <= pdrx ^ (pdrx_idx ? 2'd2 : 2'd1);
      pd_got = pd_got + 1;
    end
  end

  // ---- sends under conditions, and offers --------------------------------
  //
  // A tiny producer and consumer per pipe, as tasks would be if this bench
  // had them: `*_idx` is the entry a salt names, and a transfer toggles it.

  // `seq_route` sends odd items to `odd` from stage 1 and every item to `all`
  // from stage 2. `odd` is not drained at first: it fills with 1 and 3, the
  // even items must pass it, and 5 must hold stage 1 while 1..4 still reach
  // `all`. Then `odd` drains and everything completes in order.
  localparam ROUTE_N = 20;
  reg [1:0] rws=0, rox=0, rax=0;
  reg [15:0] re0=0, re1=0;
  wire [1:0] rr, row, raw;
  wire [31:0] rod, rad;
  seq_route rt(clk,rst_n,rws,rr,{re1,re0},row,rox,rod,raw,rax,rad);
  reg route_go=0, route_drain_odd=0;
  integer route_sent=0, route_all=0, route_odd=0;
  wire rws_idx = rws[0]^rws[1];
  wire rox_idx = rox[0]^rox[1];
  wire rax_idx = rax[0]^rax[1];
  wire [15:0] ro_item = rox_idx ? rod[31:16] : rod[15:0];
  wire [15:0] ra_item = rax_idx ? rad[31:16] : rad[15:0];
  always @(negedge clk) if (route_go) begin
    if (rws != ~rr && route_sent < ROUTE_N) begin
      if (rws_idx) re1 <= route_sent + 1; else re0 <= route_sent + 1;
      rws <= rws ^ (rws_idx ? 2'd2 : 2'd1);
      route_sent = route_sent + 1;
    end
    if (raw != rax) begin
      if (ra_item !== route_all + 1) $fatal(1,"route: `all` out of order: %0d, wanted %0d",ra_item,route_all+1);
      rax <= rax ^ (rax_idx ? 2'd2 : 2'd1);
      route_all = route_all + 1;
    end
    if (route_drain_odd && row != rox) begin
      if (ro_item !== 2*route_odd + 1) $fatal(1,"route: `odd` out of order: %0d, wanted %0d",ro_item,2*route_odd+1);
      rox <= rox ^ (rox_idx ? 2'd2 : 2'd1);
      route_odd = route_odd + 1;
    end
  end

  // `seq_blocked_early` sends to `x` from stage 0 and to `y` from stage 2.
  // `y` is held back first, so stages 1 and 2 fill; then `x` stops draining
  // and `y` starts. Stage 0 blocks on `x` -- and every item that left stage 0
  // must still reach `y`.
  localparam BE_N = 30;
  reg [1:0] bws=0, bxx=0, byx=0;
  reg [15:0] be0=0, be1=0;
  wire [1:0] br, bxw, byw;
  wire [31:0] bxd, byd;
  seq_blocked_early be(clk,rst_n,bws,br,{be1,be0},bxw,bxx,bxd,byw,byx,byd);
  reg be_go=0, be_drain_x=0, be_drain_y=0;
  reg [1:0] bxw_last=0;
  integer be_sent=0, be_x=0, be_y=0, be_x_pushes=0;
  wire bws_idx = bws[0]^bws[1];
  wire bxx_idx = bxx[0]^bxx[1];
  wire byx_idx = byx[0]^byx[1];
  wire [15:0] bx_item = bxx_idx ? bxd[31:16] : bxd[15:0];
  wire [15:0] by_item = byx_idx ? byd[31:16] : byd[15:0];
  always @(negedge clk) if (be_go) begin
    if (bxw !== bxw_last) begin be_x_pushes = be_x_pushes + 1; bxw_last = bxw; end
    if (bws != ~br && be_sent < BE_N) begin
      if (bws_idx) be1 <= be_sent + 1; else be0 <= be_sent + 1;
      bws <= bws ^ (bws_idx ? 2'd2 : 2'd1);
      be_sent = be_sent + 1;
    end
    if (be_drain_x && bxw != bxx) begin
      if (bx_item !== be_x + 1) $fatal(1,"blocked early: `x` out of order: %0d, wanted %0d",bx_item,be_x+1);
      bxx <= bxx ^ (bxx_idx ? 2'd2 : 2'd1);
      be_x = be_x + 1;
    end
    if (be_drain_y && byw != byx) begin
      if (by_item !== be_y + 1) $fatal(1,"blocked early: `y` out of order: %0d, wanted %0d",by_item,be_y+1);
      byx <= byx ^ (byx_idx ? 2'd2 : 2'd1);
      be_y = be_y + 1;
    end
  end

  // `seq_offer` offers every item to `side` and logs whether it was taken.
  // `side` drains on a slow pattern, so offers are both accepted and
  // declined; every logged `ok` must be exactly an item `side` received, in
  // order, and no offer may hold the pipeline.
  localparam OF_N = 60;
  reg [1:0] ows=0, osx=0, olx=0;
  reg [15:0] oe0=0, oe1=0;
  wire [1:0] or_, osw, olw;
  wire [31:0] osd;
  wire [63:0] old;
  seq_offer of(clk,rst_n,ows,or_,{oe1,oe0},osw,osx,osd,olw,olx,old);
  reg offer_go=0;
  integer of_sent=0, of_logged=0, of_side=0, of_accepted=0, of_declined=0, of_cycle=0;
  integer of_side_items [0:OF_N-1];
  integer of_accepted_items [0:OF_N-1];
  integer of_i;
  wire ows_idx = ows[0]^ows[1];
  wire osx_idx = osx[0]^osx[1];
  wire olx_idx = olx[0]^olx[1];
  wire [15:0] os_item = osx_idx ? osd[31:16] : osd[15:0];
  wire [31:0] ol_item = olx_idx ? old[63:32] : old[31:0];
  always @(negedge clk) if (offer_go) begin
    of_cycle = of_cycle + 1;
    if (ows != ~or_ && of_sent < OF_N) begin
      if (ows_idx) oe1 <= of_sent + 1; else oe0 <= of_sent + 1;
      ows <= ows ^ (ows_idx ? 2'd2 : 2'd1);
      of_sent = of_sent + 1;
    end
    if ((of_cycle % 5) == 0 && osw != osx) begin
      of_side_items[of_side] = os_item;
      osx <= osx ^ (osx_idx ? 2'd2 : 2'd1);
      of_side = of_side + 1;
    end
    if (olw != olx) begin
      if (ol_item[15:0] !== of_logged + 1) $fatal(1,"offer: log out of order: %0d, wanted %0d",ol_item[15:0],of_logged+1);
      if (ol_item[31:16] == 1) begin of_accepted_items[of_accepted] = ol_item[15:0]; of_accepted = of_accepted + 1; end
      else of_declined = of_declined + 1;
      olx <= olx ^ (olx_idx ? 2'd2 : 2'd1);
      of_logged = of_logged + 1;
    end
  end

  // Both sinks of the join advance together, every cycle, always.
  always @(posedge clk) begin
    if (rst_n && jsw !== jdw) $fatal(1,"join sinks moved apart: %b vs %b",jsw,jdw);
    if (rst_n && obr !== 2'b00) $fatal(1,"an empty optional input was consumed");
  end

  initial begin
    repeat(3) @(negedge clk);
    a.mem[6]=16'd123; a.mem[2]=16'd456; m.mem[0]=16'd7;
    rst_n=1;
    // Empty stages contain zero: execution assertions must remain quiet.
    repeat(6) @(negedge clk);
    input_data=32'h00020006; ws=3;

    // `a` carries 6 then 2; `b` carries 1 twice. The sums are 7 and 3, the
    // differences 5 and 1, and each pair belongs to one item.
    ja_data=32'h00020006; jb_data=32'h00010001;
    oa_data=32'h00020006;
    // `a` carries 6 then 2; `b` carries 1 and then has nothing more.
    pa_data=32'h00020006; pb_data=32'h00000001;
    repeat(4) @(negedge clk);
    // Held back one input to prove the join waits for it: nothing may leave.
    jaws=3;
    repeat(6) @(negedge clk);
    if(jsw !== 0 || jdw !== 0) $fatal(1,"the join fired without its second input");
    if(jar !== 0) $fatal(1,"the join consumed an input it could not use");
    jbws=3;
    // The optional input never arrives, so this one runs on `a` alone.
    oaws=3;
    // And here it arrives once: the first item is 6+1, the second is 2 alone.
    paws=3; pbws=1;

    repeat(20) @(negedge clk);
    if(sw !== 3 || sd !== 32'h000a000a) $fatal(1,"lexical shadowing");
    if(aw !== 3 || ad !== 32'h01c8007b) $fatal(1,"same-name BRAM address and result");
    if(mw !== 3 || md !== 32'h002a0007) $fatal(1,"read-first source order");
    if(vw !== 3 || vd !== input_data) $fatal(1,"assertion pipeline transfer");
    if(lw !== 3 || ld !== 32'h002a002a) $fatal(1,"contextual send literal");
    if(jsw !== 3 || jsd !== 32'h00030007) $fatal(1,"join sums: %h",jsd);
    if(jdw !== 3 || jdd !== 32'h00010005) $fatal(1,"join differences: %h",jdd);
    if(jar !== 3 || jbr !== 3) $fatal(1,"both inputs step together");
    if(oow !== 3 || ood !== 32'h00020006) $fatal(1,"optional input absent: %h",ood);
    if(pow !== 3 || pod !== 32'h00020007) $fatal(1,"optional input paired per item: %h",pod);
    if(pbr !== 1) $fatal(1,"the optional input gave up %b entries, not one",pbr);
    if(par !== 3) $fatal(1,"the blocking input ran to completion regardless");

    early_go=1;
    repeat(400) @(negedge clk);
    if(early_got !== EARLY_N) $fatal(1,"sends from different stages stalled: %0d of %0d joined",early_got,EARLY_N);

    side_go=1;
    repeat(600) @(negedge clk);
    if(side_got !== SIDE_N) $fatal(1,"sideband pipeline stalled: %0d of %0d",side_got,SIDE_N);
    if(side_bsteps !== side_bgot) $fatal(1,"sideband stepped %0d times for %0d samples",side_bsteps,side_bgot);
    if(side_bgot !== SIDE_B) $fatal(1,"sideband samples: %0d of %0d (offered %0d, items sent %0d, cycles %0d)",side_bgot,SIDE_B,side_bsent,side_sent,side_cycle);

    pd_go=1;
    repeat(400) @(negedge clk);
    if(pd_got !== PD_N) $fatal(1,"peek/drop pipeline stalled: %0d of %0d",pd_got,PD_N);
    if(pd_hits !== 2 || pd_bsteps !== 2) $fatal(1,"peek/drop took %0d (hits %0d), wanted 2",pd_bsteps,pd_hits);

    route_go=1;
    repeat(200) @(negedge clk);
    if(route_all !== 4) $fatal(1,"route: a full `odd` let %0d items through, wanted 4",route_all);
    if(row !== 2'b11) $fatal(1,"route: `odd` should hold 1 and 3: wsalt %b",row);
    route_drain_odd=1;
    repeat(300) @(negedge clk);
    if(route_all !== ROUTE_N || route_odd !== ROUTE_N/2) $fatal(1,"route stalled: all %0d, odd %0d",route_all,route_odd);

    be_go=1; be_drain_x=1;
    repeat(20) @(negedge clk);
    be_drain_x=0; be_drain_y=1;
    repeat(200) @(negedge clk);
    if(be_x_pushes < 6) $fatal(1,"blocked early: only %0d items left stage 0 before it blocked",be_x_pushes);
    if(be_y !== be_x_pushes) $fatal(1,"blocked early: %0d items left stage 0 but %0d reached `y`",be_x_pushes,be_y);
    be_drain_x=1;
    repeat(300) @(negedge clk);
    if(be_x !== BE_N || be_y !== BE_N) $fatal(1,"blocked early stalled: x %0d, y %0d",be_x,be_y);

    offer_go=1;
    repeat(600) @(negedge clk);
    if(of_logged !== OF_N) $fatal(1,"offer: an offer held the pipeline: %0d of %0d logged",of_logged,OF_N);
    if(of_accepted == 0 || of_declined == 0) $fatal(1,"offer: accepted %0d, declined %0d",of_accepted,of_declined);
    if(of_accepted !== of_side) $fatal(1,"offer: %0d said ok, `side` received %0d",of_accepted,of_side);
    for (of_i = 0; of_i < of_side; of_i = of_i + 1)
      if(of_accepted_items[of_i] !== of_side_items[of_i]) $fatal(1,"offer: ok item %0d is %0d, `side` received %0d",of_i,of_accepted_items[of_i],of_side_items[of_i]);
    $display("TB_PASS: sequence scoping, BRAM identities, assertions, literals, read-first, join/scatter, optional input, sends from several stages, nonblocking inputs below the head, conditional sends, blocked early sends, offers RTL");
    $finish;
  end
endmodule
