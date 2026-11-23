/// Calcule la distance entre deux points GPS (formule Haversine)
pub fn haversine_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let earth_radius_km = 6371.0;

    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();

    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);

    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    earth_radius_km * c
}

/// Trouve les points dans un rayon donné
pub fn find_within_radius(
    center_lat: f64,
    center_lon: f64,
    points: &[(f64, f64, String)],
    radius_km: f64,
) -> Vec<(f64, f64, String, f64)> {
    let mut results = Vec::new();

    for (lat, lon, name) in points {
        let distance = haversine_distance(center_lat, center_lon, *lat, *lon);
        if distance <= radius_km {
            results.push((*lat, *lon, name.clone(), distance));
        }
    }

    // Trier par distance croissante
    results.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal));
    results
}
