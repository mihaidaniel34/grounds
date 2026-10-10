// grounds: no-findings
import java.util.*;
// records generate hashCode(); AbstractList provides a matching one
record Sample(int value) {
    public boolean equals(Object other) {
        return other instanceof Sample s && value == s.value;
    }
}
class Items extends AbstractList<String> {
    public String get(int i) { return ""; }
    public int size() { return 0; }
    public boolean equals(Object o) { return super.equals(o); }
}
