---
title: Run mdkb on Kubernetes
tags: [mdkb, run, k8s]
---

# Run mdkb on Kubernetes

Deploy the daemon as a single writer (`replicas: 1`) with a ClusterIP Service.

![[01KVHJ76YHDWY5PMB7GY9B0WP6]]

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: mdkbd
spec:
  replicas: 1
  selector:
    matchLabels:
      app: mdkbd
  template:
    metadata:
      labels:
        app: mdkbd
    spec:
      containers:
        - name: mdkbd
          image: ghcr.io/example/mdkb:latest
          args: ["mdkbd", "--vault", "/vault", "--listen", "0.0.0.0:7820"]
          ports:
            - containerPort: 7820
          volumeMounts:
            - name: vault
              mountPath: /vault
      volumes:
        - name: vault
          persistentVolumeClaim:
            claimName: mdkb-vault
```
