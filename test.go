// let a = "a";
// let b = "b";
// let c = a + b;
// println c;
// println "";
//
// let a = 1;
// let b = 2;
// let c = a + b;
// println c;
// println "";
//
// fn fizzbuzz(n: int) {
//     if (n % 15 == 0) {
//         println "FizzBuzz";
//     } else if (n % 5 == 0) {
//         println "Fizz";
//     } else if (n % 3 == 0) {
//         println "Buzz";
//     } else {
//         println n;
//     }
// }
//
// let sum = 0;
//
// for (let i = 0; i < 2; i = i + 1) {
//     fizzbuzz(i);
//     sum = sum + i;
// }
// print "SUM OF 100: ";
// println sum;
//
// fn add_two_number(a:int, b:int) -> int {
//     return a + b;
// }
//
// println add_two_number(5, 15);
// println "Hello, World";

type Point {
    x:int,
    y:int
}

impl Point < Rectangle {
    fn new(x, y) {
        println x;
        println y;
    }
    fn say_something(s: String) -> String {
        return s;
    }
}

let point = Point(1,2);
println point.x;
