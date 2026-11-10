import missing.Exchange;
class Deliverer extends missing.BaseDeliverer {
    // the inner call may target the inherited findResource(List<String>)
    protected Object findResource(Exchange exchange) {
        return findResource(exchange.getRequest().getOptions().getUriPath());
    }
    int loop(int x) { return loop(x); } // expect
    // arguments typed exactly as the parameters leave no room for an inherited overload
    Object lookup(String key, java.util.List<String> path) { return lookup(key, path); } // expect
    Object all(String... keys) { return all(keys); } // expect
    <T> T first(T value) { return first(value); } // expect
}
