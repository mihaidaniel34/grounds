package jakarta.persistence; public interface EntityManager { Object createQuery(String q); Object createNativeQuery(String q); }
