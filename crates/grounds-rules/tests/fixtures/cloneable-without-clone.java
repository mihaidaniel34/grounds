class A implements Cloneable { } // expect
class B implements Cloneable { @Override public Object clone() throws CloneNotSupportedException { return super.clone(); } }
