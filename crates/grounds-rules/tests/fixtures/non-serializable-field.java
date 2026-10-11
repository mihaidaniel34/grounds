import java.io.Serializable;
import java.util.List;
final class NotSer { }
class A implements Serializable {
    private NotSer bad; // expect
    private transient NotSer ok;
    private List<String> iface;
    private String s;
    private Thread thread; // expect
}
class Holder implements java.io.Serializable {
    private Object payload;
}
