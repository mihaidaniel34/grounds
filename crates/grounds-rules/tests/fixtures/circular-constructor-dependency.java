package app;
import org.springframework.stereotype.Service;
@Service class A { A(B b) {} } // expect
@Service class B { B(C c) {} } // expect
@Service class C { C(A a) {} } // expect
@Service class D { D(A a) {} }
