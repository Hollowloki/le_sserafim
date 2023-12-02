let f;

fn f1() {
  let a = "a";
  fn f2() {
    let b = "b";
    fn f3() {
      let c = "c";
      fn f4() {
        println a;
        println b;
        println c;
      }
      f = f4;
    }
    f3();
  }
  f2();
}
f1();

f();
// out: a
// out: b
// out: c
