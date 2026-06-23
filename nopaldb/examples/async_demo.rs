use tokio::time::{Duration, sleep};

#[tokio::main]
async fn main() {
    println!("Inicio");

    let task1 = async {
        // El await es necesario para esperar a que la tarea termine. Esto ejecuta el future
        sleep(Duration::from_secs(2)).await;
        println!("Tarea 1 terminada 2s");
        "resultado de la tarea 1"
    };

    let task2 = async {
        // Al igual que task1 await espera a que termine la tarea en n segundos.
        sleep(Duration::from_secs(1)).await;
        println!("Tarea 2 terminada 1s");
        "resultado de la tarea 2"
    };

    // Async permite ejecutar tareas en paralelo, y el bloque principal espera a que todas terminen.
    let task3 = async {
        sleep(Duration::from_secs(3)).await;
        println!("Tarea 3 terminada 3s");
        "resultado de la tarea 3"
    };

    // Tokio join permite ejecutar tareas en paralelo y esperar a que todas terminen.
    // El tiempo total de ejecución es el mayor de los tiempos de las tareas.
    let (r1, r2, r3) = tokio::join!(task1, task2, task3);
    println!("Resultados: {}, {}, {}", r1, r2, r3);
    println!("Fin (total ~3s, no 6s)");
}
