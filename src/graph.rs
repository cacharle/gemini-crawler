// matrix way (good for a lot of edges) (adjacency matrix):
//    1   2   3   4
// 1  0   0   0   0
// 2  0   0   0   0
// 3  0   0   0   1
// 4  0   0   0   0
//
// one list per node way (good for a lot of nodes) (adjacency list):
// 1: []
// 2: [3]
// 3: [2]
// 4: []

#[derive(Debug, Clone)]
struct Node<T: PartialEq + Eq + Clone> {
    id: T,
    adjacent: Vec<Node<T>>,
}
#[derive(Debug)]
struct Graph<T: PartialEq + Eq + Clone>(Vec<Node<T>>);

impl<T: PartialEq + Eq + Clone> PartialEq for Node<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: PartialEq + Eq + Clone> Graph<T> {
    fn new() -> Graph<T> {
        Graph(Vec::new())
    }

    fn insert_node(&mut self, id: T) {
        self.0.push(Node { id, adjacent: Vec::new() });
    }

    fn insert_edge(&mut self, id1: T, id2: T) {
        let node1 = self.0.iter().find(|n| n.id == id1).unwrap().clone();
        let node2 = self.0.iter().find(|n| n.id == id2).unwrap().clone();
        if node1.adjacent.contains(&node2) || node2.adjacent.contains(&node1) {
            return;
        }
        for n in &mut self.0 {
            if n.id == node1.id {
                n.adjacent.push(node2.clone());
                break;
            }
        }
        for n in &mut self.0 {
            if n.id == node2.id {
                n.adjacent.push(node1.clone());
                break;
            }
        }
    }
}
