pipeline {
    agent any
    stages {
        stage('Build') {
            steps {
                sh 'make'
                script {
                    deployApp()
                }
            }
        }
    }
}
